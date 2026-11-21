use std::path::PathBuf;
use std::sync::Arc;
use parking_lot::Mutex;
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::Memory::*;

use crate::tabs::InputTarget;
use crate::terminal::buffer::{is_wide_char, TerminalBuffer};

pub const WM_APP_SESSION_DATA: u32 = WM_USER + 1;
pub const WM_APP_SESSION_STATUS: u32 = WM_USER + 2;
pub const WM_APP_LOG_STATE_CHANGED: u32 = WM_USER + 3;

const CF_UNICODETEXT: u32 = 13;
const VK_C: VIRTUAL_KEY = VIRTUAL_KEY(0x43);
const VK_V: VIRTUAL_KEY = VIRTUAL_KEY(0x56);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontConfig {
    pub font_name: String,
    pub font_size: i32,
}

impl Default for FontConfig {
    fn default() -> Self {
        Self {
            font_name: "Consolas".to_string(),
            font_size: 16,
        }
    }
}

impl FontConfig {
    pub fn parse(content: &str) -> Self {
        let mut config = Self::default();
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') || line.starts_with('[') {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                let key = k.trim();
                let val = v.trim();
                match key {
                    "font_name" if !val.is_empty() => config.font_name = val.to_string(),
                    "font_size" => {
                        if let Ok(sz) = val.parse::<i32>() {
                            config.font_size = sz.clamp(8, 72);
                        }
                    }
                    _ => {}
                }
            }
        }
        config
    }

    pub fn to_ini(&self) -> String {
        format!(
            "[Font]\r\nfont_name={}\r\nfont_size={}\r\n",
            self.font_name, self.font_size
        )
    }
}

pub fn get_font_config_path() -> PathBuf {
    super::get_config_path()
}

pub fn load_font_config() -> FontConfig {
    let path = get_font_config_path();
    if let Ok(content) = std::fs::read_to_string(&path) {
        FontConfig::parse(&content)
    } else {
        FontConfig::default()
    }
}

pub fn save_font_config(config: &FontConfig) {
    let path = get_font_config_path();
    super::save_ini_section(&path, "Font", &config.to_ini());
}

pub struct CanvasState {
    pub buffer: Option<Arc<Mutex<TerminalBuffer>>>,
    pub input_tx: Option<InputTarget>,
    pub scroll_offset: usize,
    pub char_width: i32,
    pub char_height: i32,
    pub font_size: i32,
    pub font_name: String,
    pub sel_start: Option<(usize, usize)>, // (row, col)
    pub sel_end: Option<(usize, usize)>,   // (row, col)
    pub is_dragging: bool,
    /// Cached GDI font; rebuilt only when `font_name`/`font_size` change (see `ensure_font`).
    font: HFONT,
    font_key: (String, i32),
}

impl Drop for CanvasState {
    fn drop(&mut self) {
        if !self.font.is_invalid() {
            unsafe {
                let _ = DeleteObject(self.font.into());
            }
        }
    }
}

/// Returns the cached font for the current name/size, creating it on first use or after a change.
unsafe fn ensure_font(state: &mut CanvasState) -> HFONT {
    if state.font.is_invalid() || state.font_key.0 != state.font_name || state.font_key.1 != state.font_size {
        if !state.font.is_invalid() {
            let _ = DeleteObject(state.font.into());
        }
        let name: Vec<u16> = state.font_name.encode_utf16().chain(std::iter::once(0)).collect();
        state.font = CreateFontW(
            -state.font_size,
            0,
            0,
            0,
            FW_NORMAL.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            (FIXED_PITCH.0 | FF_MODERN.0) as u32,
            PCWSTR(name.as_ptr()),
        );
        state.font_key = (state.font_name.clone(), state.font_size);
    }
    state.font
}

/// Fills `rect` with `color` without creating a brush (opaque empty text run).
unsafe fn fill_rect_color(hdc: HDC, rect: &RECT, color: u32) {
    let _ = SetBkColor(hdc, COLORREF(color));
    let _ = ExtTextOutW(hdc, rect.left, rect.top, ETO_OPAQUE, Some(rect), None, 0, None);
}

impl Default for CanvasState {
    fn default() -> Self {
        Self {
            buffer: None,
            input_tx: None,
            scroll_offset: 0,
            char_width: 9,
            char_height: 18,
            font_size: 16,
            font_name: "Consolas".to_string(),
            sel_start: None,
            sel_end: None,
            is_dragging: false,
            font: HFONT::default(),
            font_key: (String::new(), 0),
        }
    }
}

pub const CANVAS_CLASS_NAME: PCWSTR = windows::core::w!("Win32SshCanvasClass");

pub unsafe fn register_canvas_class(instance: HMODULE) -> Result<u16> {
    let wnd_class = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(canvas_wnd_proc),
        cbClsExtra: 0,
        cbWndExtra: std::mem::size_of::<*mut CanvasState>() as i32,
        hInstance: instance.into(),
        hIcon: HICON(std::ptr::null_mut()),
        hCursor: LoadCursorW(None, IDC_IBEAM)?, // I-Beam cursor for text selection
        hbrBackground: HBRUSH(COLOR_WINDOW.0 as _),
        lpszMenuName: PCWSTR::null(),
        lpszClassName: CANVAS_CLASS_NAME,
        hIconSm: HICON(std::ptr::null_mut()),
    };

    let atom = RegisterClassExW(&wnd_class);
    if atom == 0 {
        return Err(windows::core::Error::from_thread());
    }
    Ok(atom)
}

pub unsafe fn create_canvas_window(
    parent: HWND,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> Result<HWND> {
    let instance = GetModuleHandleW(None)?;
    let _ = register_canvas_class(instance);

    let font_cfg = load_font_config();
    let mut canvas_state = CanvasState::default();
    canvas_state.font_name = font_cfg.font_name;
    canvas_state.font_size = font_cfg.font_size;
    let state = Box::into_raw(Box::new(canvas_state));

    let hwnd = CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        CANVAS_CLASS_NAME,
        windows::core::w!("Terminal Canvas"),
        WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN | WS_TABSTOP | WS_VSCROLL,
        x,
        y,
        width,
        height,
        Some(parent),
        None,
        Some(instance.into()),
        Some(state as *const std::ffi::c_void),
    )?;

    SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);
    Ok(hwnd)
}

pub unsafe fn canvas_set_state(
    hwnd: HWND,
    buffer: Option<Arc<Mutex<TerminalBuffer>>>,
    input_tx: Option<InputTarget>,
) {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut CanvasState;
    if !ptr.is_null() {
        (*ptr).buffer = buffer;
        (*ptr).input_tx = input_tx;
        (*ptr).scroll_offset = 0;
        (*ptr).sel_start = None;
        (*ptr).sel_end = None;
        (*ptr).is_dragging = false;
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

pub unsafe fn canvas_set_font_size(hwnd: HWND, size: i32) {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut CanvasState;
    if !ptr.is_null() {
        (*ptr).font_size = size.clamp(8, 72);
        save_font_config(&FontConfig {
            font_name: (*ptr).font_name.clone(),
            font_size: (*ptr).font_size,
        });
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

pub unsafe fn canvas_set_font_name(hwnd: HWND, name: &str) {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut CanvasState;
    if !ptr.is_null() {
        (*ptr).font_name = name.to_string();
        save_font_config(&FontConfig {
            font_name: (*ptr).font_name.clone(),
            font_size: (*ptr).font_size,
        });
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

pub unsafe fn canvas_zoom_font(hwnd: HWND, delta: i32) {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut CanvasState;
    if !ptr.is_null() {
        let cur = (*ptr).font_size;
        (*ptr).font_size = (cur + delta).clamp(8, 72);
        save_font_config(&FontConfig {
            font_name: (*ptr).font_name.clone(),
            font_size: (*ptr).font_size,
        });
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

pub unsafe fn canvas_get_cols_rows(hwnd: HWND) -> (usize, usize) {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut CanvasState;
    let mut rect = RECT::default();
    let _ = GetClientRect(hwnd, &mut rect);
    let width = (rect.right - rect.left).max(1);
    let height = (rect.bottom - rect.top).max(1);

    if !ptr.is_null() && (*ptr).char_width > 0 && (*ptr).char_height > 0 {
        let cols = (width / (*ptr).char_width).max(10) as usize;
        let rows = (height / (*ptr).char_height).max(5) as usize;
        (cols, rows)
    } else {
        (80, 24)
    }
}

fn is_cell_selected(row: usize, col: usize, start: (usize, usize), end: (usize, usize)) -> bool {
    let ((r1, c1), (r2, c2)) = if start <= end {
        (start, end)
    } else {
        (end, start)
    };

    if row < r1 || row > r2 {
        return false;
    }
    if r1 == r2 {
        col >= c1 && col <= c2
    } else if row == r1 {
        col >= c1
    } else if row == r2 {
        col <= c2
    } else {
        true
    }
}

fn get_selected_text(state: &CanvasState) -> Option<String> {
    let (start, end) = match (state.sel_start, state.sel_end) {
        (Some(s), Some(e)) if s != e => (s, e),
        _ => return None,
    };

    let ((r1, c1), (r2, c2)) = if start <= end {
        (start, end)
    } else {
        (end, start)
    };

    let visible_lines = {
        let buf_arc = state.buffer.as_ref()?;
        let buf = buf_arc.lock();
        buf.get_visible_lines(state.scroll_offset)
    };

    let mut result = String::new();
    for row in r1..=r2 {
        if row >= visible_lines.len() {
            break;
        }
        let line = &visible_lines[row];
        let col_start = if row == r1 { c1 } else { 0 };
        let col_end = if row == r2 { c2.min(line.len().saturating_sub(1)) } else { line.len().saturating_sub(1) };

        if col_start <= col_end && col_start < line.len() {
            let row_str: String = line[col_start..=col_end]
                .iter()
                .filter(|c| !c.is_wide_continuation)
                .map(|c| c.ch)
                .collect();
            result.push_str(row_str.trim_end());
        }
        if row < r2 {
            result.push_str("\r\n");
        }
    }

    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

pub unsafe fn copy_to_clipboard(hwnd: HWND, text: &str) -> bool {
    if text.is_empty() {
        return false;
    }
    let utf16: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes_len = utf16.len() * 2;

    if OpenClipboard(Some(hwnd)).is_err() {
        return false;
    }
    let _ = EmptyClipboard();

    if let Ok(h_mem) = GlobalAlloc(GMEM_MOVEABLE, bytes_len) {
        let ptr = GlobalLock(h_mem) as *mut u16;
        if !ptr.is_null() {
            std::ptr::copy_nonoverlapping(utf16.as_ptr(), ptr, utf16.len());
            let _ = GlobalUnlock(h_mem);
            let _ = SetClipboardData(CF_UNICODETEXT, Some(HANDLE(h_mem.0)));
        }
    }

    let _ = CloseClipboard();
    true
}

pub unsafe fn paste_from_clipboard(hwnd: HWND) -> Option<String> {
    if IsClipboardFormatAvailable(CF_UNICODETEXT).is_err() {
        return None;
    }
    if OpenClipboard(Some(hwnd)).is_err() {
        return None;
    }

    let mut result = None;
    if let Ok(handle) = GetClipboardData(CF_UNICODETEXT) {
        let h_mem = HGLOBAL(handle.0);
        let ptr = GlobalLock(h_mem) as *const u16;
        if !ptr.is_null() {
            let mut len = 0;
            while *ptr.add(len) != 0 {
                len += 1;
            }
            let slice = std::slice::from_raw_parts(ptr, len);
            result = String::from_utf16(slice).ok();
            let _ = GlobalUnlock(h_mem);
        }
    }

    let _ = CloseClipboard();
    result
}

/// Returns the maximum scroll-back (in lines) of the attached buffer.
fn max_scroll_of(state: &CanvasState) -> usize {
    match state.buffer {
        Some(ref b) => {
            let buf = b.lock();
            buf.total_lines().saturating_sub(buf.rows)
        }
        None => 0,
    }
}

fn canvas_page_rows(state: &CanvasState) -> usize {
    match state.buffer {
        Some(ref b) => b.lock().rows.max(2) - 1,
        None => 1,
    }
}

/// Clamps `scroll_offset` and syncs the vertical scrollbar (offset 0 = bottom).
unsafe fn update_scrollbar(hwnd: HWND, state: &mut CanvasState) {
    let (total, rows) = match state.buffer {
        Some(ref b) => {
            let buf = b.lock();
            (buf.total_lines(), buf.rows)
        }
        None => (0, 1),
    };
    let max_scroll = total.saturating_sub(rows);
    state.scroll_offset = state.scroll_offset.min(max_scroll);
    let si = SCROLLINFO {
        cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
        fMask: SIF_RANGE | SIF_PAGE | SIF_POS | SIF_DISABLENOSCROLL,
        nMin: 0,
        nMax: total.max(rows).saturating_sub(1) as i32,
        nPage: rows as u32,
        nPos: (max_scroll - state.scroll_offset) as i32,
        nTrackPos: 0,
    };
    windows::Win32::UI::Controls::SetScrollInfo(hwnd, SB_VERT, &si, true);
}


#[allow(unused_variables)]
unsafe fn on_create(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    state_ptr: *mut CanvasState,
) -> LRESULT {
    let create_struct = &*(lparam.0 as *const CREATESTRUCTW);
    if !create_struct.lpCreateParams.is_null() {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create_struct.lpCreateParams as isize);
    }
    LRESULT(0)
}

#[allow(unused_variables)]
unsafe fn on_erase_bkgnd(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    state_ptr: *mut CanvasState,
) -> LRESULT {
    // Prevent flicker by not erasing background in WM_ERASEBKGND
    LRESULT(1)
}

#[allow(unused_variables)]
unsafe fn on_paint(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    state_ptr: *mut CanvasState,
) -> LRESULT {
    if state_ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let state = &mut *state_ptr;
    update_scrollbar(hwnd, state);

    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);

    let mut rect = RECT::default();
    let _ = GetClientRect(hwnd, &mut rect);
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;

    // Double buffering
    let mem_dc = CreateCompatibleDC(Some(hdc));
    let mem_bmp = CreateCompatibleBitmap(hdc, width, height);
    let old_bmp = SelectObject(mem_dc, mem_bmp.into());

    // Fill background with dark color (no brush needed)
    fill_rect_color(mem_dc, &rect, 0x001E1E1E);

    let font = ensure_font(state);
    let old_font = SelectObject(mem_dc, font.into());

    let mut tm = TEXTMETRICW::default();
    let _ = GetTextMetricsW(mem_dc, &mut tm);
    state.char_width = tm.tmAveCharWidth.max(1);
    state.char_height = tm.tmHeight.max(1);

    let _ = SetBkMode(mem_dc, TRANSPARENT);

    let snapshot = state.buffer.as_ref().map(|buf_lock| {
        let buf = buf_lock.lock();
        (buf.get_visible_lines(state.scroll_offset), buf.cursor_pos())
    });

    if let Some((visible_lines, (cursor_x, cursor_y))) = snapshot {

        let sel_range = match (state.sel_start, state.sel_end) {
            (Some(s), Some(e)) if s != e => Some((s, e)),
            _ => None,
        };

        for (row_idx, line) in visible_lines.iter().enumerate() {
            let y = (row_idx as i32) * state.char_height;
            if y >= height {
                break;
            }

            for (col_idx, cell) in line.iter().enumerate() {
                let x = (col_idx as i32) * state.char_width;
                if x >= width {
                    break;
                }

                if cell.is_wide_continuation {
                    continue;
                }

                let wide = is_wide_char(cell.ch);
                let cell_w = if wide { state.char_width * 2 } else { state.char_width };

                let is_selected = if let Some((s, e)) = sel_range {
                    is_cell_selected(row_idx, col_idx, s, e)
                } else {
                    false
                };

                let (cell_bg, cell_fg) = if is_selected {
                    // Highlight selection (Deep Sky Blue / Classic Terminal Highlight)
                    (0x00A06020, 0x00FFFFFF)
                } else {
                    (cell.bg_color, cell.fg_color)
                };

                // Draw background rectangle
                if cell_bg != 0x001E1E1E || is_selected {
                    let cell_rect = RECT {
                        left: x,
                        top: y,
                        right: x + cell_w,
                        bottom: y + state.char_height,
                    };
                    fill_rect_color(mem_dc, &cell_rect, cell_bg);
                }

                // Draw character
                if cell.ch != ' ' {
                    let _ = SetTextColor(mem_dc, COLORREF(cell_fg));
                    let mut ch_buf = [0u16; 2];
                    let utf16 = cell.ch.encode_utf16(&mut ch_buf);
                    let _ = TextOutW(mem_dc, x, y, utf16);
                }
            }
        }

        // Render cursor if in visible area and no scrolling
        if state.scroll_offset == 0 {
            let cur_x = (cursor_x as i32) * state.char_width;
            let cur_y = (cursor_y as i32) * state.char_height;
            let cur_rect = RECT {
                left: cur_x,
                top: cur_y,
                right: cur_x + state.char_width,
                bottom: cur_y + state.char_height,
            };
            FrameRect(mem_dc, &cur_rect, HBRUSH(GetStockObject(WHITE_BRUSH).0));
        }
    }

    // Blit buffer to screen
    let _ = BitBlt(hdc, 0, 0, width, height, Some(mem_dc), 0, 0, SRCCOPY);

    // Cleanup
    SelectObject(mem_dc, old_font); // font is cached in CanvasState; do not delete
    SelectObject(mem_dc, old_bmp);
    let _ = DeleteObject(mem_bmp.into());
    let _ = DeleteDC(mem_dc);

    let _ = EndPaint(hwnd, &ps);
    LRESULT(0)
}

#[allow(unused_variables)]
unsafe fn on_lbutton_down(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    state_ptr: *mut CanvasState,
) -> LRESULT {
    let _ = SetFocus(Some(hwnd));
    let _ = SetCapture(hwnd);

    if !state_ptr.is_null() {
        let state = &mut *state_ptr;
        let x = (lparam.0 & 0xFFFF) as i32;
        let y = ((lparam.0 >> 16) & 0xFFFF) as i32;

        if state.char_width > 0 && state.char_height > 0 {
            let col = (x / state.char_width).max(0) as usize;
            let row = (y / state.char_height).max(0) as usize;
            state.sel_start = Some((row, col));
            state.sel_end = Some((row, col));
            state.is_dragging = true;
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
    }
    LRESULT(0)
}

#[allow(unused_variables)]
unsafe fn on_mouse_move(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    state_ptr: *mut CanvasState,
) -> LRESULT {
    if !state_ptr.is_null() {
        let state = &mut *state_ptr;
        if state.is_dragging && state.char_width > 0 && state.char_height > 0 {
            let x = (lparam.0 & 0xFFFF) as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i32;
            let col = (x / state.char_width).max(0) as usize;
            let row = (y / state.char_height).max(0) as usize;
            state.sel_end = Some((row, col));
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
    }
    LRESULT(0)
}

#[allow(unused_variables)]
unsafe fn on_lbutton_up(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    state_ptr: *mut CanvasState,
) -> LRESULT {
    let _ = ReleaseCapture();
    if !state_ptr.is_null() {
        let state = &mut *state_ptr;
        state.is_dragging = false;
        if state.sel_start == state.sel_end {
            state.sel_start = None;
            state.sel_end = None;
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
    }
    LRESULT(0)
}

#[allow(unused_variables)]
unsafe fn on_rbutton_up(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    state_ptr: *mut CanvasState,
) -> LRESULT {
    let _ = SetFocus(Some(hwnd));
    if !state_ptr.is_null() {
        let state = &mut *state_ptr;
        let has_selection = get_selected_text(state).is_some();
        let has_clipboard = IsClipboardFormatAvailable(CF_UNICODETEXT).is_ok();

        if let Ok(popup_menu) = CreatePopupMenu() {
            const ID_MENU_COPY: usize = 1;
            const ID_MENU_PASTE: usize = 2;
            const ID_MENU_LOG_TOGGLE: usize = 3;

            let copy_flag = if has_selection { MF_STRING } else { MF_GRAYED };
            let paste_flag = if has_clipboard { MF_STRING } else { MF_GRAYED };
            let is_logging = state
                .buffer
                .as_ref()
                .map(|b| b.lock().is_logging())
                .unwrap_or(false);

            let _ = AppendMenuW(popup_menu, copy_flag, ID_MENU_COPY, windows::core::w!("Copy\tCtrl+C"));
            let _ = AppendMenuW(popup_menu, paste_flag, ID_MENU_PASTE, windows::core::w!("Paste\tCtrl+V"));
            let _ = AppendMenuW(popup_menu, MF_SEPARATOR, 0, PCWSTR::null());
            if is_logging {
                let _ = AppendMenuW(popup_menu, MF_STRING, ID_MENU_LOG_TOGGLE, windows::core::w!("Stop Tab Log"));
            } else {
                let log_flag = if state.buffer.is_some() { MF_STRING } else { MF_GRAYED };
                let _ = AppendMenuW(popup_menu, log_flag, ID_MENU_LOG_TOGGLE, windows::core::w!("Start Tab Log..."));
            }

            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);

            let chosen = TrackPopupMenu(
                popup_menu,
                TPM_RIGHTBUTTON | TPM_RETURNCMD,
                pt.x,
                pt.y,
                Some(0),
                hwnd,
                None,
            );
            let _ = DestroyMenu(popup_menu);

            if chosen.0 == ID_MENU_COPY as i32 {
                if let Some(text) = get_selected_text(state) {
                    copy_to_clipboard(hwnd, &text);
                    // Clear selection after copying
                    state.sel_start = None;
                    state.sel_end = None;
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
            } else if chosen.0 == ID_MENU_PASTE as i32 {
                if let Some(text) = paste_from_clipboard(hwnd) {
                    if let Some(ref tx) = state.input_tx {
                        tx.send_bytes(text.as_bytes().to_vec());
                    }
                }
            } else if chosen.0 == ID_MENU_LOG_TOGGLE as i32 {
                if is_logging {
                    if let Some(ref buf) = state.buffer {
                        buf.lock().stop_logging();
                    }
                    if let Ok(parent) = GetParent(hwnd) {
                        let _ = PostMessageW(Some(parent), WM_APP_LOG_STATE_CHANGED, WPARAM(0), LPARAM(0));
                    }
                } else if let Some(path) = crate::ui::dialogs::save_log_file_dialog(hwnd, "session.log") {
                    if let Some(ref buf) = state.buffer {
                        if let Err(err) = buf.lock().start_logging(&path) {
                            let msg: Vec<u16> = format!("Failed to open log file:\n{}", err)
                                .encode_utf16()
                                .chain(std::iter::once(0))
                                .collect();
                            let _ = MessageBoxW(
                                Some(hwnd),
                                PCWSTR(msg.as_ptr()),
                                windows::core::w!("Tab Log Error"),
                                MB_OK | MB_ICONERROR,
                            );
                        } else if let Ok(parent) = GetParent(hwnd) {
                            let _ = PostMessageW(Some(parent), WM_APP_LOG_STATE_CHANGED, WPARAM(0), LPARAM(0));
                        }
                    }
                }
            }
        }
    }
    LRESULT(0)
}

#[allow(unused_variables)]
unsafe fn on_mouse_wheel(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    state_ptr: *mut CanvasState,
) -> LRESULT {
    if !state_ptr.is_null() {
        let state = &mut *state_ptr;
        let delta = ((wparam.0 >> 16) as i16) as i32;
        let lines_to_scroll = delta / 120 * 3;
        if lines_to_scroll > 0 {
            state.scroll_offset = state.scroll_offset.saturating_add(lines_to_scroll as usize);
        } else {
            state.scroll_offset = state.scroll_offset.saturating_sub((-lines_to_scroll) as usize);
        }
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
    LRESULT(0)
}

#[allow(unused_variables)]
unsafe fn on_vscroll(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    state_ptr: *mut CanvasState,
) -> LRESULT {
    if !state_ptr.is_null() {
        let state = &mut *state_ptr;
        let max_scroll = max_scroll_of(state) as i64;
        let page = canvas_page_rows(state) as i64;
        let cur = max_scroll - state.scroll_offset as i64;
        let code = (wparam.0 & 0xFFFF) as i32;
        let new_pos = if code == SB_LINEUP.0 {
            cur - 1
        } else if code == SB_LINEDOWN.0 {
            cur + 1
        } else if code == SB_PAGEUP.0 {
            cur - page
        } else if code == SB_PAGEDOWN.0 {
            cur + page
        } else if code == SB_TOP.0 {
            0
        } else if code == SB_BOTTOM.0 {
            max_scroll
        } else if code == SB_THUMBTRACK.0 || code == SB_THUMBPOSITION.0 {
            let mut si = SCROLLINFO {
                cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                fMask: SIF_TRACKPOS,
                ..Default::default()
            };
            let _ = GetScrollInfo(hwnd, SB_VERT, &mut si);
            si.nTrackPos as i64
        } else {
            cur
        };
        let new_pos = new_pos.clamp(0, max_scroll);
        state.scroll_offset = (max_scroll - new_pos) as usize;
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
    LRESULT(0)
}

#[allow(unused_variables)]
unsafe fn on_char(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    state_ptr: *mut CanvasState,
) -> LRESULT {
    if !state_ptr.is_null() {
        let state = &mut *state_ptr;
        if state.input_tx.is_some() && state.scroll_offset != 0 {
            // Jump back to bottom on keypress (like PuTTY)
            state.scroll_offset = 0;
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
        let code = wparam.0 as u16;
        // Ignore Ctrl+C / Ctrl+V in WM_CHAR (handled in WM_KEYDOWN)
        if code == 3 || code == 22 {
            return LRESULT(0);
        }
        if let Some(ref tx) = state.input_tx {
            let ch = char::from_u32(code as u32).unwrap_or('\0');
            if ch != '\0' {
                let mut bytes = [0u8; 4];
                let s = ch.encode_utf8(&mut bytes);
                tx.send_bytes(s.as_bytes().to_vec());
            }
        }
    }
    LRESULT(0)
}

#[allow(unused_variables)]
unsafe fn on_keydown(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    state_ptr: *mut CanvasState,
) -> LRESULT {
    if !state_ptr.is_null() {
        let state = &mut *state_ptr;
        let vk = VIRTUAL_KEY(wparam.0 as u16);
        let ctrl_down = GetKeyState(VK_CONTROL.0 as i32) < 0;
        let shift_down = GetKeyState(VK_SHIFT.0 as i32) < 0;

        // App-level shortcuts (F2, F5, Ctrl+T, Ctrl+W, Ctrl+Tab, Ctrl+Shift+Tab)
        let app_cmd = if vk == VK_F2 {
            Some(crate::ui::menu::IDM_CONNECT_QUICK)
        } else if vk == VK_F5 {
            Some(crate::ui::menu::IDM_SCRIPTS_RUN)
        } else if ctrl_down && vk == VK_T {
            Some(crate::ui::menu::IDM_FILE_NEW_TAB)
        } else if ctrl_down && vk == VK_W {
            Some(crate::ui::menu::IDM_FILE_CLOSE_TAB)
        } else if ctrl_down && vk == VK_TAB {
            if shift_down {
                Some(crate::ui::menu::IDM_TABS_PREV)
            } else {
                Some(crate::ui::menu::IDM_TABS_NEXT)
            }
        } else {
            None
        };
        if let Some(cmd_id) = app_cmd {
            if let Ok(parent) = GetParent(hwnd) {
                let _ = PostMessageW(Some(parent), WM_COMMAND, WPARAM(cmd_id), LPARAM(0));
                return LRESULT(0);
            }
        }

        // Handle Ctrl + / Ctrl - / Ctrl 0 (Font Zoom)
        if ctrl_down {
            if vk == VIRTUAL_KEY(0xBB) || vk == VIRTUAL_KEY(0x6B) { // '+' or '=' or numpad '+'
                canvas_zoom_font(hwnd, 2);
                return LRESULT(0);
            } else if vk == VIRTUAL_KEY(0xBD) || vk == VIRTUAL_KEY(0x6D) { // '-' or numpad '-'
                canvas_zoom_font(hwnd, -2);
                return LRESULT(0);
            } else if vk == VIRTUAL_KEY(0x30) || vk == VIRTUAL_KEY(0x60) { // '0' or numpad '0'
                canvas_set_font_size(hwnd, 16);
                return LRESULT(0);
            }
        }
        if ctrl_down && vk == VK_C {
            if let Some(text) = get_selected_text(state) {
                copy_to_clipboard(hwnd, &text);
                state.sel_start = None;
                state.sel_end = None;
                let _ = InvalidateRect(Some(hwnd), None, false);
                return LRESULT(0);
            } else if let Some(ref tx) = state.input_tx {
                // If no selection, send standard Ctrl+C (SIGINT 0x03) to SSH
                tx.send_bytes(vec![0x03]);
                return LRESULT(0);
            }
        }

        // Handle Ctrl+V or Shift+Insert (Paste)
        if (ctrl_down && vk == VK_V) || (shift_down && vk == VK_INSERT) {
            if let Some(text) = paste_from_clipboard(hwnd) {
                if let Some(ref tx) = state.input_tx {
                    tx.send_bytes(text.as_bytes().to_vec());
                }
            }
            return LRESULT(0);
        }

        if let Some(ref tx) = state.input_tx {
            let seq: Option<&'static [u8]> = match vk {
                VK_UP => Some(b"\x1b[A"),
                VK_DOWN => Some(b"\x1b[B"),
                VK_RIGHT => Some(b"\x1b[C"),
                VK_LEFT => Some(b"\x1b[D"),
                VK_HOME => Some(b"\x1b[H"),
                VK_END => Some(b"\x1b[F"),
                VK_PRIOR => Some(b"\x1b[5~"), // Page Up
                VK_NEXT => Some(b"\x1b[6~"),  // Page Down
                VK_DELETE => Some(b"\x1b[3~"),
                _ => None,
            };
            if let Some(bytes) = seq {
                if state.scroll_offset != 0 {
                    state.scroll_offset = 0;
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                tx.send_bytes(bytes.to_vec());
                return LRESULT(0);
            }
        }
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

#[allow(unused_variables)]
unsafe fn on_ncdestroy(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    state_ptr: *mut CanvasState,
) -> LRESULT {
    if !state_ptr.is_null() {
        let _ = Box::from_raw(state_ptr);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

unsafe extern "system" fn canvas_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut CanvasState;

    match msg {
        WM_CREATE => on_create(hwnd, msg, wparam, lparam, state_ptr),
        WM_ERASEBKGND => on_erase_bkgnd(hwnd, msg, wparam, lparam, state_ptr),
        WM_PAINT => on_paint(hwnd, msg, wparam, lparam, state_ptr),
        WM_LBUTTONDOWN => on_lbutton_down(hwnd, msg, wparam, lparam, state_ptr),
        WM_MOUSEMOVE => on_mouse_move(hwnd, msg, wparam, lparam, state_ptr),
        WM_LBUTTONUP => on_lbutton_up(hwnd, msg, wparam, lparam, state_ptr),
        WM_RBUTTONUP => on_rbutton_up(hwnd, msg, wparam, lparam, state_ptr),
        WM_MOUSEWHEEL => on_mouse_wheel(hwnd, msg, wparam, lparam, state_ptr),
        WM_VSCROLL => on_vscroll(hwnd, msg, wparam, lparam, state_ptr),
        WM_CHAR => on_char(hwnd, msg, wparam, lparam, state_ptr),
        WM_KEYDOWN => on_keydown(hwnd, msg, wparam, lparam, state_ptr),
        WM_NCDESTROY => on_ncdestroy(hwnd, msg, wparam, lparam, state_ptr),
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
