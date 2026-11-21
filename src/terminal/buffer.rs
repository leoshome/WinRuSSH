use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalCell {
    pub ch: char,
    pub fg_color: u32,
    pub bg_color: u32,
    pub bold: bool,
    pub is_wide_continuation: bool,
}

impl Default for TerminalCell {
    fn default() -> Self {
        Self {
            ch: ' ',
            fg_color: 0x00CCCCCC, // Light gray
            bg_color: 0x001E1E1E, // Dark gray (#1E1E1E)
            bold: false,
            is_wide_continuation: false,
        }
    }
}

pub fn is_wide_char(ch: char) -> bool {
    let cp = ch as u32;
    (0x1100..=0x115F).contains(&cp)
        || (0x2E80..=0xA4CF).contains(&cp)
        || (0xAC00..=0xD7A3).contains(&cp)
        || (0xF900..=0xFAFF).contains(&cp)
        || (0xFE10..=0xFE19).contains(&cp)
        || (0xFE30..=0xFE6F).contains(&cp)
        || (0xFF00..=0xFF60).contains(&cp)
        || (0xFFE0..=0xFFE6).contains(&cp)
        || (0x20000..=0x2FA1F).contains(&cp)
}

#[derive(Debug, PartialEq, Eq)]
enum ParserState {
    Normal,
    Esc,
    Csi,
}

pub struct TerminalBuffer {
    pub cols: usize,
    pub rows: usize,
    pub max_history: usize,
    lines: Vec<Vec<TerminalCell>>,
    cursor_x: usize,
    cursor_y: usize,
    current_fg: u32,
    current_bg: u32,
    current_bold: bool,
    parser_state: ParserState,
    csi_params: Vec<u32>,
    current_param: Option<u32>,
    utf8_buf: Vec<u8>,
    log_writer: Option<BufWriter<File>>,
    log_path: Option<PathBuf>,
}

impl TerminalBuffer {
    pub fn new(cols: usize, rows: usize, max_history: usize) -> Self {
        let default_line = vec![TerminalCell::default(); cols];
        let lines = vec![default_line; rows];
        Self {
            cols,
            rows,
            max_history,
            lines,
            cursor_x: 0,
            cursor_y: 0,
            current_fg: 0x00CCCCCC,
            current_bg: 0x001E1E1E,
            current_bold: false,
            parser_state: ParserState::Normal,
            csi_params: Vec::new(),
            current_param: None,
            utf8_buf: Vec::with_capacity(4),
            log_writer: None,
            log_path: None,
        }
    }

    pub fn start_logging(&mut self, path: impl AsRef<Path>) -> std::io::Result<()> {
        self.stop_logging();
        let path_ref = path.as_ref();
        let file = File::create(path_ref)?;
        self.log_writer = Some(BufWriter::new(file));
        self.log_path = Some(path_ref.to_path_buf());
        Ok(())
    }

    pub fn stop_logging(&mut self) {
        if let Some(mut writer) = self.log_writer.take() {
            let _ = writer.flush();
        }
        self.log_path = None;
    }

    pub fn is_logging(&self) -> bool {
        self.log_writer.is_some()
    }

    pub fn log_path(&self) -> Option<&Path> {
        self.log_path.as_deref()
    }

    pub fn cursor_pos(&self) -> (usize, usize) {
        (self.cursor_x, self.cursor_y)
    }

    pub fn total_lines(&self) -> usize {
        self.lines.len()
    }

    pub fn resize(&mut self, new_cols: usize, new_rows: usize) {
        if new_cols == 0 || new_rows == 0 {
            return;
        }
        let cursor_abs = self.active_screen_row_index();
        self.cols = new_cols;
        self.rows = new_rows;

        // Grow shorter lines to match new_cols, but avoid truncating existing content
        // when the window temporarily narrows (e.g. resize or minimize).
        for line in &mut self.lines {
            if line.len() < new_cols {
                line.resize(new_cols, TerminalCell::default());
            }
        }

        // Trim trailing blank lines below the cursor when shrinking so unused
        // bottom rows don't become phantom scrollback lines.
        while self.lines.len() > new_rows && self.lines.len() - 1 > cursor_abs {
            let is_blank = self
                .lines
                .last()
                .map(|l| l.iter().all(|c| c.ch == ' '))
                .unwrap_or(false);
            if !is_blank {
                break;
            }
            self.lines.pop();
        }

        // Ensure we have at least new_rows lines
        while self.lines.len() < new_rows {
            self.lines.push(vec![TerminalCell::default(); new_cols]);
        }

        if self.cursor_x >= new_cols {
            self.cursor_x = new_cols - 1;
        }
        let new_screen_start = self.lines.len().saturating_sub(new_rows);
        self.cursor_y = cursor_abs
            .saturating_sub(new_screen_start)
            .min(new_rows - 1);
    }

    pub fn get_visible_lines(&self, scroll_offset: usize) -> Vec<Vec<TerminalCell>> {
        let total = self.lines.len();
        if total <= self.rows {
            return self.lines.clone();
        }

        let max_scroll = total - self.rows;
        let actual_scroll = scroll_offset.min(max_scroll);
        let start = total - self.rows - actual_scroll;
        self.lines[start..start + self.rows].to_vec()
    }

    fn new_cell(&self, ch: char) -> TerminalCell {
        TerminalCell {
            ch,
            fg_color: self.current_fg,
            bg_color: self.current_bg,
            bold: self.current_bold,
            is_wide_continuation: false,
        }
    }

    fn line_feed(&mut self) {
        if self.cursor_y + 1 >= self.rows {
            // Scroll up: add a new line at bottom
            self.lines.push(vec![TerminalCell::default(); self.cols]);
            if self.lines.len() > self.max_history + self.rows {
                self.lines.remove(0);
            }
        } else {
            self.cursor_y += 1;
        }
    }

    fn active_screen_row_index(&self) -> usize {
        if self.lines.len() >= self.rows {
            self.lines.len() - self.rows + self.cursor_y
        } else {
            self.cursor_y
        }
    }

    fn put_char(&mut self, ch: char) {
        let wide = is_wide_char(ch);
        let needed_cols = if wide { 2 } else { 1 };

        if self.cursor_x + needed_cols > self.cols {
            self.cursor_x = 0;
            self.line_feed();
        }

        let row_idx = self.active_screen_row_index();
        if row_idx < self.lines.len() && self.cursor_x < self.cols {
            self.lines[row_idx][self.cursor_x] = self.new_cell(ch);
            if wide && self.cursor_x + 1 < self.cols {
                self.lines[row_idx][self.cursor_x + 1] = TerminalCell {
                    ch: ' ',
                    fg_color: self.current_fg,
                    bg_color: self.current_bg,
                    bold: self.current_bold,
                    is_wide_continuation: true,
                };
                self.cursor_x += 2;
            } else {
                self.cursor_x += 1;
            }
        }
    }

    pub fn write_bytes(&mut self, data: &[u8]) {
        if let Some(ref mut writer) = self.log_writer {
            let _ = writer.write_all(data);
            let _ = writer.flush();
        }
        for &b in data {
            match self.parser_state {
                ParserState::Normal => {
                    match b {
                        0x1B => {
                            self.utf8_buf.clear();
                            self.parser_state = ParserState::Esc;
                        }
                        b'\r' => {
                            self.utf8_buf.clear();
                            self.cursor_x = 0;
                        }
                        b'\n' => {
                            self.utf8_buf.clear();
                            self.line_feed();
                        }
                        0x08 => {
                            self.utf8_buf.clear();
                            if self.cursor_x > 0 {
                                self.cursor_x -= 1;
                            }
                        }
                        b'\t' => {
                            self.utf8_buf.clear();
                            let next_tab = (self.cursor_x / 8 + 1) * 8;
                            self.cursor_x = next_tab.min(self.cols.saturating_sub(1));
                        }
                        b if (0x20..=0x7E).contains(&b) => {
                            self.utf8_buf.clear();
                            self.put_char(b as char);
                        }
                        b if b >= 0x80 => {
                            self.utf8_buf.push(b);
                            let parsed = std::str::from_utf8(&self.utf8_buf).map(|s| s.chars().collect::<Vec<char>>());
                            match parsed {
                                Ok(chars) => {
                                    self.utf8_buf.clear();
                                    for ch in chars {
                                        self.put_char(ch);
                                    }
                                }
                                Err(e) => {
                                    if e.error_len().is_some() {
                                        self.utf8_buf.clear();
                                    } else if self.utf8_buf.len() >= 4 {
                                        self.utf8_buf.clear();
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
                ParserState::Esc => {
                    match b {
                        b'[' => {
                            self.parser_state = ParserState::Csi;
                            self.csi_params.clear();
                            self.current_param = None;
                        }
                        _ => {
                            self.parser_state = ParserState::Normal;
                        }
                    }
                }
                ParserState::Csi => {
                    match b {
                        b'0'..=b'9' => {
                            let digit = (b - b'0') as u32;
                            self.current_param = Some(self.current_param.unwrap_or(0) * 10 + digit);
                        }
                        b';' => {
                            self.csi_params.push(self.current_param.unwrap_or(0));
                            self.current_param = None;
                        }
                        b'm' => {
                            // SGR Color / Font attribute
                            if let Some(param) = self.current_param {
                                self.csi_params.push(param);
                            }
                            if self.csi_params.is_empty() {
                                self.csi_params.push(0);
                            }
                            self.apply_sgr();
                            self.parser_state = ParserState::Normal;
                        }
                        b'K' => {
                            // Erase in line
                            let mode = self.current_param.unwrap_or(0);
                            self.erase_in_line(mode);
                            self.parser_state = ParserState::Normal;
                        }
                        b'J' => {
                            // Erase in display
                            let mode = self.current_param.unwrap_or(0);
                            self.erase_in_display(mode);
                            self.parser_state = ParserState::Normal;
                        }
                        b'H' | b'f' => {
                            // Cursor position
                            if let Some(param) = self.current_param {
                                self.csi_params.push(param);
                            }
                            let row = self.csi_params.first().copied().unwrap_or(1).max(1) as usize - 1;
                            let col = self.csi_params.get(1).copied().unwrap_or(1).max(1) as usize - 1;
                            self.cursor_y = row.min(self.rows.saturating_sub(1));
                            self.cursor_x = col.min(self.cols.saturating_sub(1));
                            self.parser_state = ParserState::Normal;
                        }
                        b'A' => {
                            let n = self.current_param.unwrap_or(1).max(1) as usize;
                            self.cursor_y = self.cursor_y.saturating_sub(n);
                            self.parser_state = ParserState::Normal;
                        }
                        b'B' => {
                            let n = self.current_param.unwrap_or(1).max(1) as usize;
                            self.cursor_y = (self.cursor_y + n).min(self.rows.saturating_sub(1));
                            self.parser_state = ParserState::Normal;
                        }
                        b'C' => {
                            let n = self.current_param.unwrap_or(1).max(1) as usize;
                            self.cursor_x = (self.cursor_x + n).min(self.cols.saturating_sub(1));
                            self.parser_state = ParserState::Normal;
                        }
                        b'D' => {
                            let n = self.current_param.unwrap_or(1).max(1) as usize;
                            self.cursor_x = self.cursor_x.saturating_sub(n);
                            self.parser_state = ParserState::Normal;
                        }
                        _ => {
                            // End of CSI or unsupported sequence
                            self.parser_state = ParserState::Normal;
                        }
                    }
                }
            }
        }
    }

    fn erase_in_line(&mut self, mode: u32) {
        let row_idx = self.active_screen_row_index();
        if row_idx >= self.lines.len() {
            return;
        }
        let blank = TerminalCell::default();
        match mode {
            0 => {
                // Clear from cursor to end of line
                for x in self.cursor_x..self.cols {
                    self.lines[row_idx][x] = blank;
                }
            }
            1 => {
                // Clear from beginning of line to cursor
                for x in 0..=self.cursor_x.min(self.cols.saturating_sub(1)) {
                    self.lines[row_idx][x] = blank;
                }
            }
            2 => {
                // Clear entire line
                for x in 0..self.cols {
                    self.lines[row_idx][x] = blank;
                }
            }
            _ => {}
        }
    }

    fn erase_in_display(&mut self, mode: u32) {
        let blank = TerminalCell::default();
        let total = self.lines.len();
        let screen_start = total.saturating_sub(self.rows);
        match mode {
            2 => {
                // Clear entire screen
                for y in screen_start..total {
                    for x in 0..self.cols {
                        self.lines[y][x] = blank;
                    }
                }
                self.cursor_x = 0;
                self.cursor_y = 0;
            }
            _ => {}
        }
    }

    fn apply_sgr(&mut self) {
        for &code in &self.csi_params {
            match code {
                0 => {
                    self.current_fg = 0x00CCCCCC;
                    self.current_bg = 0x001E1E1E;
                    self.current_bold = false;
                }
                1 => self.current_bold = true,
                22 => self.current_bold = false,
                30 => self.current_fg = 0x00000000, // Black
                31 => self.current_fg = 0x000000FF, // Red (Windows COLORREF: 0x00BBGGRR)
                32 => self.current_fg = 0x0000FF00, // Green
                33 => self.current_fg = 0x0000FFFF, // Yellow
                34 => self.current_fg = 0x00FF0000, // Blue
                35 => self.current_fg = 0x00FF00FF, // Magenta
                36 => self.current_fg = 0x00FFFF00, // Cyan
                37 => self.current_fg = 0x00CCCCCC, // White
                39 => self.current_fg = 0x00CCCCCC, // Default FG
                40 => self.current_bg = 0x00000000,
                41 => self.current_bg = 0x00000088,
                42 => self.current_bg = 0x00008800,
                43 => self.current_bg = 0x00008888,
                44 => self.current_bg = 0x00880000,
                45 => self.current_bg = 0x00880088,
                46 => self.current_bg = 0x00888800,
                47 => self.current_bg = 0x00CCCCCC,
                49 => self.current_bg = 0x001E1E1E, // Default BG
                90 => self.current_fg = 0x00767676,
                91 => self.current_fg = 0x005555FF, // Bright red
                92 => self.current_fg = 0x0055FF55,
                93 => self.current_fg = 0x0055FFFF,
                94 => self.current_fg = 0x00FF5555,
                95 => self.current_fg = 0x00FF55FF,
                96 => self.current_fg = 0x00FFFF55,
                97 => self.current_fg = 0x00FFFFFF,
                _ => {}
            }
        }
    }
}
