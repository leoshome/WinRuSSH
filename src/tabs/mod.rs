use std::sync::Arc;
use parking_lot::Mutex;
use crossbeam_channel::Sender;
use windows::Win32::Foundation::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::Win32::UI::Controls::*;
use windows::core::*;

use crate::terminal::buffer::TerminalBuffer;
use crate::session::SessionCommand;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabStatus {
    Idle,
    Connecting,
    RunningLua,
    Connected,
    Disconnected,
}

impl TabStatus {
    /// Returns a Win32 COLORREF (`0x00BBGGRR`) status dot color, or `None` when idle.
    pub fn dot_color(self) -> Option<u32> {
        match self {
            TabStatus::Connected => Some(0x005EC522),                           // RGB(34, 197, 94) green
            TabStatus::Disconnected => Some(0x004444EF),                        // RGB(239, 68, 68) red
            TabStatus::Connecting | TabStatus::RunningLua => Some(0x0008B3EA), // RGB(234, 179, 8) yellow
            TabStatus::Idle => None,
        }
    }
}

impl std::fmt::Display for TabStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            TabStatus::Idle => "Idle",
            TabStatus::Connecting => "Connecting",
            TabStatus::RunningLua => "Running Lua",
            TabStatus::Connected => "Connected",
            TabStatus::Disconnected => "Idle / Disconnected",
        })
    }
}

/// Sends TCM_INSERTITEMW / TCM_SETITEMW with `title` as the item text.
unsafe fn send_tab_text(hwnd: HWND, msg: u32, index: usize, title: &str) {
    let mut title_utf16: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
    let tc_item = TCITEMW {
        mask: TCIF_TEXT,
        pszText: PWSTR(title_utf16.as_mut_ptr()),
        ..Default::default()
    };
    let _ = SendMessageW(hwnd, msg, Some(WPARAM(index)), Some(LPARAM(&tc_item as *const _ as isize)));
}

#[derive(Clone)]
pub enum InputTarget {
    Ssh(Sender<SessionCommand>),
    Raw(Sender<Vec<u8>>),
}

impl InputTarget {
    pub fn send_bytes(&self, bytes: Vec<u8>) {
        match self {
            Self::Ssh(tx) => {
                let _ = tx.send(SessionCommand::Input(bytes));
            }
            Self::Raw(tx) => {
                let _ = tx.send(bytes);
            }
        }
    }
}

pub struct SessionTab {
    pub id: usize,
    pub title: String,
    pub buffer: Arc<Mutex<TerminalBuffer>>,
    pub input_tx: Option<InputTarget>,
    pub command_tx: Option<Sender<SessionCommand>>,
    pub status: TabStatus,
}

pub struct TabManager {
    hwnd_tab: Option<HWND>,
    tabs: Vec<SessionTab>,
    active_idx: usize,
    next_id: usize,
}

impl TabManager {
    pub fn new(hwnd_tab: HWND) -> Self {
        Self {
            hwnd_tab: Some(hwnd_tab),
            tabs: Vec::new(),
            active_idx: 0,
            next_id: 0,
        }
    }

    pub fn new_headless() -> Self {
        Self {
            hwnd_tab: None,
            tabs: Vec::new(),
            active_idx: 0,
            next_id: 0,
        }
    }

    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    pub fn active_index(&self) -> usize {
        self.active_idx
    }

    pub fn active_tab(&self) -> Option<&SessionTab> {
        self.tabs.get(self.active_idx)
    }

    pub fn active_tab_mut(&mut self) -> Option<&mut SessionTab> {
        self.tabs.get_mut(self.active_idx)
    }

    pub fn get_tab(&self, index: usize) -> Option<&SessionTab> {
        self.tabs.get(index)
    }

    pub fn get_tab_mut_by_id(&mut self, id: usize) -> Option<&mut SessionTab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    pub fn add_tab(&mut self, title: &str) -> usize {
        let id = self.next_id;
        self.next_id += 1;

        let buffer = Arc::new(Mutex::new(TerminalBuffer::new(80, 24, 5000)));
        let tab = SessionTab {
            id,
            title: title.to_string(),
            buffer,
            input_tx: None,
            command_tx: None,
            status: TabStatus::Idle,
        };

        let new_idx = self.tabs.len();
        self.tabs.push(tab);

        if let Some(hwnd) = self.hwnd_tab {
            unsafe { send_tab_text(hwnd, TCM_INSERTITEMW, new_idx, title) };
        }

        id
    }

    pub fn close_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }

        if let Some(tab) = self.tabs.get(index) {
            tab.buffer.lock().stop_logging();
            if let Some(ref tx) = tab.command_tx {
                let _ = tx.send(SessionCommand::Disconnect);
            }
        }

        self.tabs.remove(index);

        if let Some(hwnd) = self.hwnd_tab {
            unsafe {
                let _ = SendMessageW(hwnd, TCM_DELETEITEM, Some(WPARAM(index)), Some(LPARAM(0)));
            }
        }

        if self.tabs.is_empty() {
            self.active_idx = 0;
        } else if self.active_idx >= self.tabs.len() {
            self.active_idx = self.tabs.len() - 1;
        }
    }

    pub fn switch_to(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.active_idx = index;
            if let Some(hwnd) = self.hwnd_tab {
                unsafe {
                    let _ = SendMessageW(hwnd, TCM_SETCURSEL, Some(WPARAM(index)), Some(LPARAM(0)));
                }
            }
        }
    }

    pub fn rename_tab(&mut self, index: usize, title: &str) {
        if let Some(tab) = self.tabs.get_mut(index) {
            tab.title = title.to_string();
            if let Some(hwnd) = self.hwnd_tab {
                unsafe { send_tab_text(hwnd, TCM_SETITEMW, index, title) };
            }
        }
    }

    pub fn sync_from_win32(&mut self) {
        if let Some(hwnd) = self.hwnd_tab {
            unsafe {
                let cur = SendMessageW(hwnd, TCM_GETCURSEL, Some(WPARAM(0)), Some(LPARAM(0))).0 as usize;
                if cur < self.tabs.len() {
                    self.active_idx = cur;
                }
            }
        }
    }
}

