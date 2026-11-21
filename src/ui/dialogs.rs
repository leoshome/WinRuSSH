use std::ffi::c_void;
use std::path::PathBuf;
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::{COLOR_BTNFACE, HBRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::EM_SETSEL;
use windows::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, GetSaveFileNameW, OPENFILENAMEW, OFN_FILEMUSTEXIST, OFN_OVERWRITEPROMPT,
    OFN_PATHMUSTEXIST,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::ok_or_abort;
use crate::session::{SshAuth, SshConfig};
use super::win::{create_child, create_ui_font};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickConnectConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub log_enabled: bool,
    pub log_path: String,
}

#[derive(Debug, Clone)]
pub struct QuickConnectResult {
    pub ssh: SshConfig,
    pub log_path: Option<String>,
}

impl Default for QuickConnectConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 22,
            username: "root".to_string(),
            password: String::new(),
            log_enabled: false,
            log_path: String::new(),
        }
    }
}

impl QuickConnectConfig {
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
                    "host" => config.host = val.to_string(),
                    "port" => {
                        if let Ok(p) = val.parse::<u16>() {
                            config.port = p;
                        }
                    }
                    "username" => config.username = val.to_string(),
                    "password" => config.password = val.to_string(),
                    "log_enabled" => config.log_enabled = val == "1" || val.eq_ignore_ascii_case("true"),
                    "log_path" => config.log_path = val.to_string(),
                    _ => {}
                }
            }
        }
        config
    }

    pub fn to_ini(&self) -> String {
        format!(
            "[QuickConnect]\r\nhost={}\r\nport={}\r\nusername={}\r\npassword={}\r\nlog_enabled={}\r\nlog_path={}\r\n",
            self.host,
            self.port,
            self.username,
            self.password,
            if self.log_enabled { "1" } else { "0" },
            self.log_path
        )
    }
}

pub fn get_quick_connect_config_path() -> PathBuf {
    super::get_config_path()
}

pub fn load_quick_connect_config() -> QuickConnectConfig {
    let path = get_quick_connect_config_path();
    if let Ok(content) = std::fs::read_to_string(&path) {
        QuickConnectConfig::parse(&content)
    } else {
        QuickConnectConfig::default()
    }
}

pub fn save_quick_connect_config(config: &QuickConnectConfig) {
    let path = get_quick_connect_config_path();
    super::save_ini_section(&path, "QuickConnect", &config.to_ini());
}

pub fn open_script_file_dialog(owner: HWND) -> Option<String> {
    unsafe {
        let mut file_buf = [0u16; 1024];
        let filter: Vec<u16> = "Lua Scripts (*.lua)\0*.lua\0All Files (*.*)\0*.*\0\0"
            .encode_utf16()
            .collect();

        let mut ofn = OPENFILENAMEW {
            lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
            hwndOwner: owner,
            lpstrFilter: PCWSTR(filter.as_ptr()),
            lpstrFile: PWSTR(file_buf.as_mut_ptr()),
            nMaxFile: file_buf.len() as u32,
            Flags: OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST,
            ..Default::default()
        };

        if GetOpenFileNameW(&mut ofn).as_bool() {
            let len = file_buf.iter().position(|&c| c == 0).unwrap_or(file_buf.len());
            String::from_utf16(&file_buf[..len]).ok()
        } else {
            None
        }
    }
}

pub fn save_log_file_dialog(owner: HWND, default_name: &str) -> Option<String> {
    unsafe {
        let mut file_buf = [0u16; 1024];
        let default_utf16: Vec<u16> = default_name.encode_utf16().collect();
        let copy_len = default_utf16.len().min(file_buf.len().saturating_sub(1));
        file_buf[..copy_len].copy_from_slice(&default_utf16[..copy_len]);

        let filter: Vec<u16> = "Log Files (*.log;*.txt)\0*.log;*.txt\0All Files (*.*)\0*.*\0\0"
            .encode_utf16()
            .collect();
        let def_ext: Vec<u16> = "log\0".encode_utf16().collect();

        let mut ofn = OPENFILENAMEW {
            lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
            hwndOwner: owner,
            lpstrFilter: PCWSTR(filter.as_ptr()),
            lpstrFile: PWSTR(file_buf.as_mut_ptr()),
            nMaxFile: file_buf.len() as u32,
            lpstrDefExt: PCWSTR(def_ext.as_ptr()),
            Flags: OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST,
            ..Default::default()
        };

        if GetSaveFileNameW(&mut ofn).as_bool() {
            let len = file_buf.iter().position(|&c| c == 0).unwrap_or(file_buf.len());
            String::from_utf16(&file_buf[..len]).ok()
        } else {
            None
        }
    }
}

// Quick Connect dialog state
struct ConnectDialogData {
    config: QuickConnectConfig,
    result: Option<QuickConnectResult>,
    h_host: HWND,
    h_port: HWND,
    h_user: HWND,
    h_pass: HWND,
    h_log_chk: HWND,
    h_log_path: HWND,
    h_log_browse: HWND,
}

const ID_DLG_CONNECT: usize = 1;
const ID_DLG_CANCEL: usize = 2;
const ID_DLG_LOG_CHK: usize = 3;
const ID_DLG_LOG_BROWSE: usize = 4;

pub fn show_quick_connect_dialog(owner: HWND) -> Option<QuickConnectResult> {
    unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        let class_name = windows::core::w!("QuickConnectDlgClass");

        let wnd_class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(connect_dlg_proc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).ok()?,
            hbrBackground: HBRUSH((COLOR_BTNFACE.0 + 1) as _),
            lpszClassName: class_name,
            ..Default::default()
        };
        let _ = RegisterClassExW(&wnd_class);

        let saved_cfg = load_quick_connect_config();

        let mut data = ConnectDialogData {
            config: saved_cfg,
            result: None,
            h_host: HWND(std::ptr::null_mut()),
            h_port: HWND(std::ptr::null_mut()),
            h_user: HWND(std::ptr::null_mut()),
            h_pass: HWND(std::ptr::null_mut()),
            h_log_chk: HWND(std::ptr::null_mut()),
            h_log_path: HWND(std::ptr::null_mut()),
            h_log_browse: HWND(std::ptr::null_mut()),
        };

        let width = 370;
        let height = 275;
        let mut owner_rect = RECT::default();
        let _ = GetWindowRect(owner, &mut owner_rect);
        let x = owner_rect.left + (owner_rect.right - owner_rect.left - width) / 2;
        let y = owner_rect.top + (owner_rect.bottom - owner_rect.top - height) / 2;

        let hwnd_dlg = CreateWindowExW(
            WS_EX_CONTROLPARENT,
            class_name,
            windows::core::w!("Quick Connect"),
            WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_VISIBLE,
            x,
            y,
            width,
            height,
            Some(owner),
            None,
            Some(instance.into()),
            Some(&mut data as *mut _ as *const c_void),
        ).ok()?;

        let _ = EnableWindow(owner, false);

        // Focus Host input by default and select all text
        let _ = SetFocus(Some(data.h_host));
        let _ = SendMessageW(data.h_host, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));

        let mut msg = MSG::default();
        while IsWindow(Some(hwnd_dlg)).as_bool() && GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if !IsDialogMessageW(hwnd_dlg, &msg).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        let _ = EnableWindow(owner, true);
        let _ = SetForegroundWindow(owner);

        data.result
    }
}

unsafe extern "system" fn connect_dlg_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let data_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ConnectDialogData;

    match msg {
        WM_CREATE => {
            let create_struct = &*(lparam.0 as *const CREATESTRUCTW);
            if !create_struct.lpCreateParams.is_null() {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, create_struct.lpCreateParams as isize);
                let data = &mut *(create_struct.lpCreateParams as *mut ConnectDialogData);
                let instance = create_struct.hInstance;

                let host_w: Vec<u16> = data.config.host.encode_utf16().chain(std::iter::once(0)).collect();
                let port_w: Vec<u16> = data.config.port.to_string().encode_utf16().chain(std::iter::once(0)).collect();
                let user_w: Vec<u16> = data.config.username.encode_utf16().chain(std::iter::once(0)).collect();
                let pass_w: Vec<u16> = data.config.password.encode_utf16().chain(std::iter::once(0)).collect();
                let log_path_w: Vec<u16> = data.config.log_path.encode_utf16().chain(std::iter::once(0)).collect();

                let h = hwnd;
                let mk = |class: PCWSTR, text: PCWSTR, style: u32, rect: (i32, i32, i32, i32), id: usize| {
                    create_child(h, instance, class, text, WINDOW_STYLE(style), rect, id)
                };
                let tab = WS_TABSTOP.0;
                let edit = WS_BORDER.0 | tab;
                let st = w!("STATIC");
                let ed = w!("EDIT");
                let btn = w!("BUTTON");
                let none = PCWSTR::null();

                // Label & edit rows
                let h_lbl_host = ok_or_abort!(mk(st, w!("Host:"), 0, (20, 20, 65, 20), 0));
                data.h_host = ok_or_abort!(mk(ed, PCWSTR(host_w.as_ptr()), edit, (90, 18, 160, 22), 0));
                let h_lbl_port = ok_or_abort!(mk(st, w!("Port:"), 0, (260, 20, 35, 20), 0));
                data.h_port = ok_or_abort!(mk(ed, PCWSTR(port_w.as_ptr()), edit, (298, 18, 47, 22), 0));
                let h_lbl_user = ok_or_abort!(mk(st, w!("User:"), 0, (20, 55, 65, 20), 0));
                data.h_user = ok_or_abort!(mk(ed, PCWSTR(user_w.as_ptr()), edit, (90, 53, 255, 22), 0));
                let h_lbl_pass = ok_or_abort!(mk(st, w!("Password:"), 0, (20, 90, 68, 20), 0));
                data.h_pass = ok_or_abort!(mk(ed, PCWSTR(pass_w.as_ptr()), edit | ES_PASSWORD as u32, (90, 88, 255, 22), 0));

                // Log to file checkbox, path edit, and browse button
                data.h_log_chk = ok_or_abort!(mk(btn, w!("Log:"), tab | BS_AUTOCHECKBOX as u32, (20, 125, 60, 20), ID_DLG_LOG_CHK));
                data.h_log_path = ok_or_abort!(mk(ed, PCWSTR(log_path_w.as_ptr()), edit | ES_AUTOHSCROLL as u32, (90, 123, 215, 22), 0));
                data.h_log_browse = ok_or_abort!(mk(btn, w!("..."), tab, (312, 123, 33, 22), ID_DLG_LOG_BROWSE));

                let log_checked = data.config.log_enabled;
                let _ = SendMessageW(data.h_log_chk, BM_SETCHECK, Some(WPARAM(if log_checked { 1 } else { 0 })), Some(LPARAM(0)));
                let _ = EnableWindow(data.h_log_path, log_checked);
                let _ = EnableWindow(data.h_log_browse, log_checked);

                let h_btn_ok = ok_or_abort!(mk(btn, w!("Connect"), tab | BS_DEFPUSHBUTTON as u32, (150, 175, 90, 30), ID_DLG_CONNECT));
                let h_btn_cancel = ok_or_abort!(mk(btn, w!("Cancel"), tab, (255, 175, 90, 30), ID_DLG_CANCEL));
                let _ = none;

                // Set clean Segoe UI font on all dialog controls
                let dlg_font = create_ui_font(-13);
                for &ctrl in &[
                    h_lbl_host,
                    data.h_host,
                    h_lbl_port,
                    data.h_port,
                    h_lbl_user,
                    data.h_user,
                    h_lbl_pass,
                    data.h_pass,
                    data.h_log_chk,
                    data.h_log_path,
                    data.h_log_browse,
                    h_btn_ok,
                    h_btn_cancel,
                ] {
                    let _ = SendMessageW(ctrl, WM_SETFONT, Some(WPARAM(dlg_font.0 as usize)), Some(LPARAM(1)));
                }
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = wparam.0 as usize & 0xFFFF;
            if id == ID_DLG_LOG_CHK {
                if !data_ptr.is_null() {
                    let data = &mut *data_ptr;
                    let checked = SendMessageW(data.h_log_chk, BM_GETCHECK, Some(WPARAM(0)), Some(LPARAM(0))).0 == 1;
                    let _ = EnableWindow(data.h_log_path, checked);
                    let _ = EnableWindow(data.h_log_browse, checked);
                    if checked && get_edit_text(data.h_log_path).trim().is_empty() {
                        if let Some(path) = save_log_file_dialog(hwnd, "session.log") {
                            let path_w: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
                            let _ = SetWindowTextW(data.h_log_path, PCWSTR(path_w.as_ptr()));
                        } else {
                            let _ = SendMessageW(data.h_log_chk, BM_SETCHECK, Some(WPARAM(0)), Some(LPARAM(0)));
                            let _ = EnableWindow(data.h_log_path, false);
                            let _ = EnableWindow(data.h_log_browse, false);
                        }
                    }
                }
            } else if id == ID_DLG_LOG_BROWSE {
                if !data_ptr.is_null() {
                    let data = &mut *data_ptr;
                    if let Some(path) = save_log_file_dialog(hwnd, "session.log") {
                        let path_w: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
                        let _ = SetWindowTextW(data.h_log_path, PCWSTR(path_w.as_ptr()));
                        let _ = SendMessageW(data.h_log_chk, BM_SETCHECK, Some(WPARAM(1)), Some(LPARAM(0)));
                        let _ = EnableWindow(data.h_log_path, true);
                        let _ = EnableWindow(data.h_log_browse, true);
                    }
                }
            } else if id == ID_DLG_CONNECT {
                // Connect
                if !data_ptr.is_null() {
                    let data = &mut *data_ptr;
                    let host = get_edit_text(data.h_host).trim().to_string();
                    let port_str = get_edit_text(data.h_port);
                    let port = port_str.trim().parse::<u16>().unwrap_or(22);
                    let username = get_edit_text(data.h_user);
                    let password = get_edit_text(data.h_pass);
                    let log_enabled = SendMessageW(data.h_log_chk, BM_GETCHECK, Some(WPARAM(0)), Some(LPARAM(0))).0 == 1;
                    let log_path_str = get_edit_text(data.h_log_path).trim().to_string();

                    // Persist connection parameters to memory file
                    save_quick_connect_config(&QuickConnectConfig {
                        host: host.clone(),
                        port,
                        username: username.clone(),
                        password: password.clone(),
                        log_enabled,
                        log_path: log_path_str.clone(),
                    });

                    data.result = Some(QuickConnectResult {
                        ssh: SshConfig {
                            host,
                            port,
                            username,
                            auth: SshAuth::Password(password),
                        },
                        log_path: if log_enabled && !log_path_str.is_empty() {
                            Some(log_path_str)
                        } else {
                            None
                        },
                    });
                }
                let _ = DestroyWindow(hwnd);
            } else if id == ID_DLG_CANCEL {
                // Cancel
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn get_edit_text(hwnd: HWND) -> String {
    let len = GetWindowTextLengthW(hwnd);
    if len > 0 {
        let mut buf = vec![0u16; (len + 1) as usize];
        GetWindowTextW(hwnd, &mut buf);
        String::from_utf16_lossy(&buf[..len as usize])
    } else {
        String::new()
    }
}

