#![windows_subsystem = "windows"]

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use winrussh::scripting::spawn_lua_worker;
use winrussh::session::{spawn_ssh_worker, SendHwnd, SessionCommand};
use winrussh::ok_or_abort;
use winrussh::tabs::{InputTarget, TabManager, TabStatus};
use winrussh::ui::win::{create_child, create_ui_font};
use winrussh::ui::canvas::{
    canvas_get_cols_rows, canvas_set_font_name, canvas_set_font_size, canvas_set_state,
    canvas_zoom_font, create_canvas_window, WM_APP_LOG_STATE_CHANGED, WM_APP_SESSION_DATA,
    WM_APP_SESSION_STATUS,
};
use winrussh::ui::dialogs::{open_script_file_dialog, show_quick_connect_dialog};
use winrussh::ui::menu::*;

struct AppState {
    tab_manager: TabManager,
    hwnd_tab: HWND,
    hwnd_new_tab_btn: HWND,
    hwnd_canvas: HWND,
    hwnd_status: HWND,
    ui_font: HFONT,
    tab_font: HFONT,
    /// Repaint throttle: true while the throttle timer is running.
    repaint_timer_on: bool,
    /// New data arrived since the last repaint while the timer was running.
    repaint_dirty: bool,
}

const WM_APP_CLI_LAUNCH: u32 = WM_APP + 100;
const TIMER_REPAINT: usize = 1;
const REPAINT_INTERVAL_MS: u32 = 16;

const MAIN_CLASS_NAME: PCWSTR = windows::core::w!("Win32SshConsoleMainClass");

fn main() -> Result<()> {
    unsafe {
        // Enable Per-Monitor High-DPI Awareness v2 to eliminate blurry scaling
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

        let instance = GetModuleHandleW(None)?;

        // Initialize common controls (SysTabControl32, msctls_statusbar32)
        let icce = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_TAB_CLASSES,
        };
        let _ = InitCommonControlsEx(&icce);

        let wnd_class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(main_wnd_proc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hbrBackground: HBRUSH(COLOR_WINDOW.0 as _),
            lpszClassName: MAIN_CLASS_NAME,
            ..Default::default()
        };
        let _ = RegisterClassExW(&wnd_class);

        let hwnd_main = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            MAIN_CLASS_NAME,
            windows::core::w!("WinRuSSH - Native Windows SSH Console & Lua Automation"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1000,
            650,
            None,
            None,
            Some(instance.into()),
            None,
        )?;

        use windows::Win32::UI::Input::KeyboardAndMouse::{
            GetKeyState, VK_CONTROL, VK_SHIFT, VK_TAB,
        };
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            // Handle Ctrl+Tab / Ctrl+Shift+Tab globally regardless of which child control has focus
            if msg.message == WM_KEYDOWN
                && msg.wParam.0 as u16 == VK_TAB.0
                && GetKeyState(VK_CONTROL.0 as i32) < 0
            {
                let cmd = if GetKeyState(VK_SHIFT.0 as i32) < 0 {
                    IDM_TABS_PREV
                } else {
                    IDM_TABS_NEXT
                };
                let _ = PostMessageW(Some(hwnd_main), WM_COMMAND, WPARAM(cmd), LPARAM(0));
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        Ok(())
    }
}

/// Open a new tab and run a Lua script in it, exposing `args` as Lua `arg[1..]`.
unsafe fn launch_script(state: &mut AppState, hwnd: HWND, path: String, args: Vec<String>) {
    let script_name = std::path::Path::new(&path)
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("Script");
    let title = format!("Lua: {}", script_name);
    let _id = state.tab_manager.add_tab(&title);
    let new_idx = state.tab_manager.tab_count() - 1;
    state.tab_manager.switch_to(new_idx);

    if let Some(tab) = state.tab_manager.active_tab_mut() {
        let (cols, rows) = canvas_get_cols_rows(state.hwnd_canvas);
        tab.buffer.lock().resize(cols, rows);

        let (input_tx, _handle) =
            spawn_lua_worker(path, args, tab.buffer.clone(), Some(SendHwnd(hwnd)), tab.id);

        tab.input_tx = Some(InputTarget::Raw(input_tx));
        tab.status = TabStatus::RunningLua;
    }
    sync_active_tab(state);
}

/// Owner-draw the bottom status bar with a colored connection dot (green/red/yellow) and text.
unsafe fn draw_status_bar(dis: &DRAWITEMSTRUCT, tab: &winrussh::tabs::SessionTab) {
    let hdc = dis.hDC;
    let rc = dis.rcItem;
    let mut text_left = rc.left + 6;

    if let Some(dot_rgb) = tab.status.dot_color() {
        let dot_size = 10;
        let dot_y = rc.top + ((rc.bottom - rc.top) - dot_size) / 2;
        let brush = CreateSolidBrush(COLORREF(dot_rgb));
        let pen = CreatePen(PS_SOLID, 1, COLORREF(dot_rgb));
        let old_brush = SelectObject(hdc, brush.into());
        let old_pen = SelectObject(hdc, pen.into());
        let _ = Ellipse(hdc, text_left, dot_y, text_left + dot_size, dot_y + dot_size);
        SelectObject(hdc, old_brush);
        SelectObject(hdc, old_pen);
        let _ = DeleteObject(brush.into());
        let _ = DeleteObject(pen.into());
        text_left += dot_size + 6;
    }

    let status_text = format_status_text(tab);
    let mut text_rc = RECT { left: text_left, top: rc.top, right: rc.right - 4, bottom: rc.bottom };
    let _ = SetBkMode(hdc, TRANSPARENT);
    let _ = SetTextColor(hdc, COLORREF(GetSysColor(COLOR_WINDOWTEXT)));
    let mut wide: Vec<u16> = status_text.encode_utf16().collect();
    let _ = DrawTextW(hdc, &mut wide, &mut text_rc, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
}

unsafe extern "system" fn main_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut AppState;

    match msg {
        WM_CREATE => {
            let instance = GetModuleHandleW(None).unwrap();

            // Set main menu
            if let Ok(menu) = build_main_menu() {
                let _ = SetMenu(hwnd, Some(menu));
            }

            let ui_font = create_ui_font(-13);
            let tab_font = create_ui_font(-16);
            let inst = HINSTANCE(instance.0);

            // Tab control (single-line mode shows right-side scroll arrows on overflow)
            let hwnd_tab = ok_or_abort!(create_child(
                hwnd, inst, WC_TABCONTROLW, windows::core::w!(""),
                WS_CLIPSIBLINGS | WINDOW_STYLE(TCS_SINGLELINE | TCS_HOTTRACK),
                (0, 0, 966, 36), 1001,
            ));
            let _ = SendMessageW(hwnd_tab, WM_SETFONT, Some(WPARAM(tab_font.0 as usize)), Some(LPARAM(1)));
            let _ = SendMessageW(hwnd_tab, TCM_SETPADDING, Some(WPARAM(0)), Some(LPARAM((6 << 16) | 12)));

            // "+" New Tab button next to tab bar
            let hwnd_new_tab_btn = ok_or_abort!(create_child(
                hwnd, inst, windows::core::w!("BUTTON"), windows::core::w!("+"),
                WS_CLIPSIBLINGS | WINDOW_STYLE(BS_PUSHBUTTON as u32),
                (968, 2, 30, 32), IDM_FILE_NEW_TAB as usize,
            ));
            let _ = SendMessageW(hwnd_new_tab_btn, WM_SETFONT, Some(WPARAM(tab_font.0 as usize)), Some(LPARAM(1)));

            // Terminal canvas
            let hwnd_canvas = ok_or_abort!(create_canvas_window(hwnd, 0, 36, 1000, 574));

            // Status bar
            let hwnd_status = ok_or_abort!(create_child(
                hwnd, inst, STATUSCLASSNAMEW,
                windows::core::w!("Ready | Press F2 to Connect, F5 to Run Script"),
                WS_CLIPSIBLINGS,
                (0, 610, 1000, 24), 1002,
            ));
            let _ = SendMessageW(hwnd_status, WM_SETFONT, Some(WPARAM(ui_font.0 as usize)), Some(LPARAM(1)));

            let mut tab_manager = TabManager::new(hwnd_tab);
            let _tab_id = tab_manager.add_tab("Tab 1 (Welcome)");

            if let Some(tab) = tab_manager.active_tab() {
                {
                    let mut b = tab.buffer.lock();
                    b.write_bytes(b"\x1b[36m===============================================================\r\n");
                    b.write_bytes(b"  WinRuSSH - Native Windows SSH Console (Rust + windows-rs)\r\n");
                    b.write_bytes(b"===============================================================\x1b[0m\r\n\r\n");
                    b.write_bytes(b"Quick Start:\r\n");
                    b.write_bytes(b"  * [F2] or Connect -> Quick Connect to start an SSH session\r\n");
                    b.write_bytes(b"  * [F5] or Scripts -> Run Lua Script... to run Expect automation\r\n");
                    b.write_bytes(b"  * [Ctrl+T] New Tab | [Ctrl+W] Close Tab | [Ctrl+Tab] Switch\r\n\r\n");
                    b.write_bytes(b"Ready.\r\n");
                }
                canvas_set_state(hwnd_canvas, Some(tab.buffer.clone()), tab.input_tx.clone());
            }

            let state = Box::into_raw(Box::new(AppState {
                tab_manager,
                hwnd_tab,
                hwnd_new_tab_btn,
                hwnd_canvas,
                hwnd_status,
                ui_font,
                tab_font,
                repaint_timer_on: false,
                repaint_dirty: false,
            }));
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);

            // Defer CLI script launch until window creation has fully finished
            let _ = PostMessageW(Some(hwnd), WM_APP_CLI_LAUNCH, WPARAM(0), LPARAM(0));

            LRESULT(0)
        }
        WM_APP_CLI_LAUNCH => {
            // CLI: winrussh.exe [--script] <script.lua> [args...]
            if !state_ptr.is_null() {
                let mut cli = std::env::args().skip(1);
                if let Some(first) = cli.next() {
                    let path = if first == "--script" { cli.next() } else { Some(first) };
                    if let Some(path) = path {
                        launch_script(&mut *state_ptr, hwnd, path, cli.collect());
                    }
                }
            }
            LRESULT(0)
        }
        WM_SIZE => {
            let width = (lparam.0 & 0xFFFF) as i32;
            let height = ((lparam.0 >> 16) & 0xFFFF) as i32;

            if wparam.0 as u32 == SIZE_MINIMIZED || width == 0 || height == 0 {
                return LRESULT(0);
            }

            if !state_ptr.is_null() {
                let state = &mut *state_ptr;
                let tab_h = 36;
                let btn_w = 30;
                let status_h = 24;
                let canvas_h = (height - tab_h - status_h).max(50);

                let _ = SetWindowPos(state.hwnd_tab, None, 0, 0, (width - btn_w - 4).max(50), tab_h, SWP_NOZORDER);
                let _ = SetWindowPos(state.hwnd_new_tab_btn, None, (width - btn_w - 2).max(52), 2, btn_w, tab_h - 4, SWP_NOZORDER);
                let _ = SetWindowPos(state.hwnd_canvas, None, 0, tab_h, width, canvas_h, SWP_NOZORDER);
                let _ = SetWindowPos(state.hwnd_status, None, 0, height - status_h, width, status_h, SWP_NOZORDER);

                resize_active_tab(state);
            }
            LRESULT(0)
        }
        WM_DRAWITEM => {
            if !state_ptr.is_null() && wparam.0 == 1002 {
                let state = &*state_ptr;
                let dis = &*(lparam.0 as *const DRAWITEMSTRUCT);
                if let Some(tab) = state.tab_manager.active_tab() {
                    draw_status_bar(dis, tab);
                    return LRESULT(1);
                }
            }
            LRESULT(0)
        }
        WM_NOTIFY => {
            let nmhdr = &*(lparam.0 as *const NMHDR);
            if !state_ptr.is_null() && nmhdr.code == TCN_SELCHANGE {
                let state = &mut *state_ptr;
                state.tab_manager.sync_from_win32();
                sync_active_tab(state);
                let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(state.hwnd_canvas));
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let cmd_id = wparam.0 as usize & 0xFFFF;
            if !state_ptr.is_null() {
                let state = &mut *state_ptr;
                match cmd_id {
                    IDM_FILE_NEW_TAB => {
                        let count = state.tab_manager.tab_count() + 1;
                        let title = format!("Tab {}", count);
                        let _id = state.tab_manager.add_tab(&title);
                        state.tab_manager.switch_to(state.tab_manager.tab_count() - 1);
                        sync_active_tab(state);
                        let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(state.hwnd_canvas));
                    }
                    IDM_FILE_CLOSE_TAB => {
                        if state.tab_manager.tab_count() > 1 {
                            let active = state.tab_manager.active_index();
                            state.tab_manager.close_tab(active);
                            sync_active_tab(state);
                        }
                    }
                    IDM_FILE_EXIT => {
                        let _ = DestroyWindow(hwnd);
                    }
                    IDM_CONNECT_QUICK => {
                        if let Some(res) = show_quick_connect_dialog(hwnd) {
                            let cfg = res.ssh;
                            let title = cfg.host.clone();

                            let can_reuse_active = state
                                .tab_manager
                                .active_tab()
                                .map(|t| t.status == TabStatus::Idle)
                                .unwrap_or(false);

                            if can_reuse_active {
                                let idx = state.tab_manager.active_index();
                                state.tab_manager.rename_tab(idx, &title);
                            } else {
                                let _id = state.tab_manager.add_tab(&title);
                                let new_idx = state.tab_manager.tab_count() - 1;
                                state.tab_manager.switch_to(new_idx);
                            }

                            if let Some(tab) = state.tab_manager.active_tab_mut() {
                                let (cols, rows) = canvas_get_cols_rows(state.hwnd_canvas);
                                {
                                    let mut buf = tab.buffer.lock();
                                    buf.resize(cols, rows);
                                    if let Some(ref log_path) = res.log_path {
                                        if let Err(err) = buf.start_logging(log_path) {
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
                                        }
                                    }
                                }

                                let (cmd_tx, _handle) = spawn_ssh_worker(
                                    cfg,
                                    tab.buffer.clone(),
                                    Some(SendHwnd(hwnd)),
                                    tab.id,
                                );

                                tab.input_tx = Some(InputTarget::Ssh(cmd_tx.clone()));
                                tab.command_tx = Some(cmd_tx);
                                tab.status = TabStatus::Connecting;
                            }
                            sync_active_tab(state);
                            let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(state.hwnd_canvas));
                        }
                    }
                    IDM_SCRIPTS_RUN => {
                        if let Some(path) = open_script_file_dialog(hwnd) {
                            launch_script(state, hwnd, path, Vec::new());
                        }
                    }
                    IDM_TABS_PREV => {
                        let count = state.tab_manager.tab_count();
                        if count > 1 {
                            let cur = state.tab_manager.active_index();
                            let prev = if cur == 0 { count - 1 } else { cur - 1 };
                            state.tab_manager.switch_to(prev);
                            sync_active_tab(state);
                        }
                    }
                    IDM_TABS_NEXT => {
                        let count = state.tab_manager.tab_count();
                        if count > 1 {
                            let next = (state.tab_manager.active_index() + 1) % count;
                            state.tab_manager.switch_to(next);
                            sync_active_tab(state);
                        }
                    }
                    IDM_FONT_ZOOM_IN => {
                        canvas_zoom_font(state.hwnd_canvas, 2);
                        resize_active_tab(state);
                    }
                    IDM_FONT_ZOOM_OUT => {
                        canvas_zoom_font(state.hwnd_canvas, -2);
                        resize_active_tab(state);
                    }
                    IDM_FONT_RESET => {
                        canvas_set_font_size(state.hwnd_canvas, 16);
                        resize_active_tab(state);
                    }
                    IDM_FONT_SIZE_12 => {
                        canvas_set_font_size(state.hwnd_canvas, 12);
                        resize_active_tab(state);
                    }
                    IDM_FONT_SIZE_14 => {
                        canvas_set_font_size(state.hwnd_canvas, 14);
                        resize_active_tab(state);
                    }
                    IDM_FONT_SIZE_16 => {
                        canvas_set_font_size(state.hwnd_canvas, 16);
                        resize_active_tab(state);
                    }
                    IDM_FONT_SIZE_18 => {
                        canvas_set_font_size(state.hwnd_canvas, 18);
                        resize_active_tab(state);
                    }
                    IDM_FONT_SIZE_20 => {
                        canvas_set_font_size(state.hwnd_canvas, 20);
                        resize_active_tab(state);
                    }
                    IDM_FONT_SIZE_24 => {
                        canvas_set_font_size(state.hwnd_canvas, 24);
                        resize_active_tab(state);
                    }
                    IDM_FONT_FAMILY_CONSOLAS => {
                        canvas_set_font_name(state.hwnd_canvas, "Consolas");
                        resize_active_tab(state);
                    }
                    IDM_FONT_FAMILY_CASCADIA => {
                        canvas_set_font_name(state.hwnd_canvas, "Cascadia Mono");
                        resize_active_tab(state);
                    }
                    IDM_FONT_FAMILY_COURIER => {
                        canvas_set_font_name(state.hwnd_canvas, "Courier New");
                        resize_active_tab(state);
                    }
                    IDM_FONT_FAMILY_LUCIDA => {
                        canvas_set_font_name(state.hwnd_canvas, "Lucida Console");
                        resize_active_tab(state);
                    }
                    IDM_HELP_SHORTCUTS => {
                        let _ = MessageBoxW(
                            Some(hwnd),
                            windows::core::w!("Keyboard Shortcuts\r\n\r\nMouse Left Drag\tSelect text\r\nMouse Right Click\tCopy / Paste / Tab Log menu\r\nCtrl+C\tCopy selection (or send SIGINT)\r\nCtrl+V / Shift+Insert\tPaste\r\nCtrl+ + / Ctrl+ =\tZoom in\r\nCtrl+ -\tZoom out\r\nCtrl+0\tReset font size\r\nF2\tQuick Connect\r\nF5\tRun Lua script\r\nCtrl+T\tNew tab\r\nCtrl+W\tClose tab\r\nCtrl+Tab\tNext tab\r\nCtrl+Shift+Tab\tPrevious tab\r\nMouse Wheel\tScroll history\r\n\r\nScript Usage\r\n\r\nF5 or Scripts > Run Lua Script...\r\nCommand line: winrussh.exe <script.lua> [args...]\r\nIn Lua: arg[1..] are the extra arguments.\r\nAPI: ssh.connect / expect / send / sleep / read\r\nSee scripts\\ for examples."),
                            windows::core::w!("Keyboard Shortcuts & Script Usage"),
                            MB_OK | MB_ICONINFORMATION,
                        );
                    }
                    IDM_HELP_ABOUT => {
                        let _ = MessageBoxW(
                            Some(hwnd),
                            windows::core::w!("WinRuSSH\r\nNative Windows SSH Console & Lua Automation\r\nVersion 0.1.0\r\n\r\nBuilt with Rust, windows-rs, libssh2, and Lua 5.4."),
                            windows::core::w!("About WinRuSSH"),
                            MB_OK | MB_ICONINFORMATION,
                        );
                    }
                    _ => {}
                }
            }
            LRESULT(0)
        }
        WM_APP_SESSION_DATA => {
            let tab_id = wparam.0 as usize;
            if !state_ptr.is_null() {
                let state = &mut *state_ptr;
                if let Some(active_tab) = state.tab_manager.active_tab() {
                    if active_tab.id == tab_id {
                        if state.repaint_timer_on {
                            // Throttled: the timer will repaint at most every 16ms.
                            state.repaint_dirty = true;
                        } else {
                            // Leading edge: repaint immediately so typing echo stays snappy.
                            let _ = InvalidateRect(Some(state.hwnd_canvas), None, false);
                            state.repaint_timer_on = true;
                            state.repaint_dirty = false;
                            SetTimer(Some(hwnd), TIMER_REPAINT, REPAINT_INTERVAL_MS, None);
                        }
                    }
                }
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_REPAINT => {
            if !state_ptr.is_null() {
                let state = &mut *state_ptr;
                if state.repaint_dirty {
                    state.repaint_dirty = false;
                    let _ = InvalidateRect(Some(state.hwnd_canvas), None, false);
                } else {
                    let _ = KillTimer(Some(hwnd), TIMER_REPAINT);
                    state.repaint_timer_on = false;
                }
            }
            LRESULT(0)
        }
        WM_APP_SESSION_STATUS => {
            let tab_id = wparam.0 as usize;
            let status_code = lparam.0;
            if !state_ptr.is_null() {
                let state = &mut *state_ptr;
                if let Some(tab) = state.tab_manager.get_tab_mut_by_id(tab_id) {
                    tab.status = if status_code == 1 {
                        TabStatus::Connected
                    } else {
                        TabStatus::Disconnected
                    };
                }
                if let Some(active_tab) = state.tab_manager.active_tab() {
                    if active_tab.id == tab_id {
                        update_status_bar(state.hwnd_status, active_tab);
                    }
                }
            }
            LRESULT(0)
        }
        WM_APP_LOG_STATE_CHANGED => {
            if !state_ptr.is_null() {
                let state = &*state_ptr;
                if let Some(active_tab) = state.tab_manager.active_tab() {
                    update_status_bar(state.hwnd_status, active_tab);
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            if !state_ptr.is_null() {
                let _ = KillTimer(Some(hwnd), TIMER_REPAINT);
                let state = Box::from_raw(state_ptr);
                let _ = DeleteObject(state.ui_font.into());
                let _ = DeleteObject(state.tab_font.into());
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn sync_active_tab(state: &AppState) {
    if let Some(tab) = state.tab_manager.active_tab() {
        canvas_set_state(state.hwnd_canvas, Some(tab.buffer.clone()), tab.input_tx.clone());
        update_status_bar(state.hwnd_status, tab);
    }
}

unsafe fn resize_active_tab(state: &mut AppState) {
    let (cols, rows) = canvas_get_cols_rows(state.hwnd_canvas);
    if let Some(tab) = state.tab_manager.active_tab_mut() {
        tab.buffer.lock().resize(cols, rows);
        if let Some(ref tx) = tab.command_tx {
            let _ = tx.send(SessionCommand::Resize {
                cols: cols as u32,
                rows: rows as u32,
            });
        }
    }
}

fn format_status_text(tab: &winrussh::tabs::SessionTab) -> String {
    let log_suffix = {
        let buf = tab.buffer.lock();
        if let Some(path) = buf.log_path() {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("active");
            format!(" | [LOG: {}]", name)
        } else {
            String::new()
        }
    };
    format!("{} | Status: {}{}", tab.title, tab.status, log_suffix)
}

unsafe fn update_status_bar(hwnd_status: HWND, _tab: &winrussh::tabs::SessionTab) {
    let _ = SendMessageW(hwnd_status, SB_SETTEXTW, Some(WPARAM(SBT_OWNERDRAW as usize)), Some(LPARAM(0)));
}


