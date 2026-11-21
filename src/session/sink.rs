use std::sync::Arc;

use parking_lot::Mutex;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::terminal::buffer::TerminalBuffer;
use crate::ui::canvas::{WM_APP_SESSION_DATA, WM_APP_SESSION_STATUS};

#[derive(Clone, Copy, Debug)]
pub struct SendHwnd(pub HWND);
unsafe impl Send for SendHwnd {}
unsafe impl Sync for SendHwnd {}

/// Single place that writes into a tab's terminal buffer and notifies the UI thread.
#[derive(Clone)]
pub struct UiSink {
    buffer: Option<Arc<Mutex<TerminalBuffer>>>,
    hwnd: Option<SendHwnd>,
    tab_id: usize,
}

impl UiSink {
    pub fn new(
        buffer: Option<Arc<Mutex<TerminalBuffer>>>,
        hwnd: Option<SendHwnd>,
        tab_id: usize,
    ) -> Self {
        Self { buffer, hwnd, tab_id }
    }

    pub fn has_buffer(&self) -> bool {
        self.buffer.is_some()
    }

    /// (cols, rows) of the attached buffer, or 80x24 when headless.
    pub fn size(&self) -> (u32, u32) {
        match &self.buffer {
            Some(b) => {
                let b = b.lock();
                (b.cols as u32, b.rows as u32)
            }
            None => (80, 24),
        }
    }

    pub fn write_bytes(&self, bytes: &[u8]) {
        if let Some(buf) = &self.buffer {
            buf.lock().write_bytes(bytes);
            self.post(WM_APP_SESSION_DATA, 0);
        }
    }

    pub fn write(&self, text: &str) {
        self.write_bytes(text.as_bytes());
    }

    /// Writes `msg` followed by CRLF.
    pub fn log(&self, msg: &str) {
        self.write(&format!("{}\r\n", msg));
    }

    pub fn error(&self, msg: &str) {
        self.log(&format!("\x1b[31m[ERROR] {}\x1b[0m", msg));
    }

    pub fn warn(&self, msg: &str) {
        self.log(&format!("\x1b[33m[WARN] {}\x1b[0m", msg));
    }

    pub fn notify_status(&self, connected: bool) {
        self.post(WM_APP_SESSION_STATUS, connected as isize);
    }

    fn post(&self, msg: u32, lparam: isize) {
        if let Some(h) = self.hwnd {
            unsafe {
                let _ = PostMessageW(Some(h.0), msg, WPARAM(self.tab_id), LPARAM(lparam));
            }
        }
    }
}
