use windows::core::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const IDM_FILE_NEW_TAB: usize = 101;
pub const IDM_FILE_CLOSE_TAB: usize = 102;
pub const IDM_FILE_EXIT: usize = 103;

pub const IDM_CONNECT_QUICK: usize = 201;
pub const IDM_CONNECT_DISCONNECT: usize = 202;

pub const IDM_SCRIPTS_RUN: usize = 301;
pub const IDM_SCRIPTS_STOP: usize = 302;

pub const IDM_TABS_PREV: usize = 401;
pub const IDM_TABS_NEXT: usize = 402;

pub const IDM_FONT_ZOOM_IN: usize = 601;
pub const IDM_FONT_ZOOM_OUT: usize = 602;
pub const IDM_FONT_RESET: usize = 603;
pub const IDM_FONT_SIZE_12: usize = 612;
pub const IDM_FONT_SIZE_14: usize = 614;
pub const IDM_FONT_SIZE_16: usize = 616;
pub const IDM_FONT_SIZE_18: usize = 618;
pub const IDM_FONT_SIZE_20: usize = 620;
pub const IDM_FONT_SIZE_24: usize = 624;
pub const IDM_FONT_FAMILY_CONSOLAS: usize = 631;
pub const IDM_FONT_FAMILY_CASCADIA: usize = 632;
pub const IDM_FONT_FAMILY_COURIER: usize = 633;
pub const IDM_FONT_FAMILY_LUCIDA: usize = 634;

pub const IDM_HELP_ABOUT: usize = 501;
pub const IDM_HELP_SHORTCUTS: usize = 502;

pub unsafe fn build_main_menu() -> Result<HMENU> {
    let menu = CreateMenu()?;

    // File
    let file_menu = CreatePopupMenu()?;
    AppendMenuW(file_menu, MF_STRING, IDM_FILE_NEW_TAB, windows::core::w!("New Tab\tCtrl+T"))?;
    AppendMenuW(file_menu, MF_STRING, IDM_FILE_CLOSE_TAB, windows::core::w!("Close Tab\tCtrl+W"))?;
    AppendMenuW(file_menu, MF_SEPARATOR, 0, PCWSTR::null())?;
    AppendMenuW(file_menu, MF_STRING, IDM_FILE_EXIT, windows::core::w!("Exit\tAlt+F4"))?;
    AppendMenuW(menu, MF_POPUP, file_menu.0 as usize, windows::core::w!("&File"))?;

    // Connect
    let connect_menu = CreatePopupMenu()?;
    AppendMenuW(connect_menu, MF_STRING, IDM_CONNECT_QUICK, windows::core::w!("Quick Connect...\tF2"))?;
    AppendMenuW(connect_menu, MF_STRING, IDM_CONNECT_DISCONNECT, windows::core::w!("Disconnect"))?;
    AppendMenuW(menu, MF_POPUP, connect_menu.0 as usize, windows::core::w!("&Connect"))?;

    // Scripts
    let scripts_menu = CreatePopupMenu()?;
    AppendMenuW(scripts_menu, MF_STRING, IDM_SCRIPTS_RUN, windows::core::w!("Run Lua Script...\tF5"))?;
    AppendMenuW(scripts_menu, MF_STRING, IDM_SCRIPTS_STOP, windows::core::w!("Stop Script"))?;
    AppendMenuW(menu, MF_POPUP, scripts_menu.0 as usize, windows::core::w!("&Scripts"))?;

    // Tabs
    let tabs_menu = CreatePopupMenu()?;
    AppendMenuW(tabs_menu, MF_STRING, IDM_TABS_PREV, windows::core::w!("Previous Tab\tCtrl+Shift+Tab"))?;
    AppendMenuW(tabs_menu, MF_STRING, IDM_TABS_NEXT, windows::core::w!("Next Tab\tCtrl+Tab"))?;
    AppendMenuW(menu, MF_POPUP, tabs_menu.0 as usize, windows::core::w!("&Tabs"))?;

    // Font
    let font_menu = CreatePopupMenu()?;
    AppendMenuW(font_menu, MF_STRING, IDM_FONT_ZOOM_IN, windows::core::w!("Zoom In\tCtrl++"))?;
    AppendMenuW(font_menu, MF_STRING, IDM_FONT_ZOOM_OUT, windows::core::w!("Zoom Out\tCtrl+-"))?;
    AppendMenuW(font_menu, MF_STRING, IDM_FONT_RESET, windows::core::w!("Reset Size (16px)\tCtrl+0"))?;
    AppendMenuW(font_menu, MF_SEPARATOR, 0, PCWSTR::null())?;

    // Font Sizes
    AppendMenuW(font_menu, MF_STRING, IDM_FONT_SIZE_12, windows::core::w!("12 px"))?;
    AppendMenuW(font_menu, MF_STRING, IDM_FONT_SIZE_14, windows::core::w!("14 px"))?;
    AppendMenuW(font_menu, MF_STRING, IDM_FONT_SIZE_16, windows::core::w!("16 px (Default)"))?;
    AppendMenuW(font_menu, MF_STRING, IDM_FONT_SIZE_18, windows::core::w!("18 px"))?;
    AppendMenuW(font_menu, MF_STRING, IDM_FONT_SIZE_20, windows::core::w!("20 px"))?;
    AppendMenuW(font_menu, MF_STRING, IDM_FONT_SIZE_24, windows::core::w!("24 px"))?;
    AppendMenuW(font_menu, MF_SEPARATOR, 0, PCWSTR::null())?;

    // Font Families
    AppendMenuW(font_menu, MF_STRING, IDM_FONT_FAMILY_CONSOLAS, windows::core::w!("Consolas"))?;
    AppendMenuW(font_menu, MF_STRING, IDM_FONT_FAMILY_CASCADIA, windows::core::w!("Cascadia Mono"))?;
    AppendMenuW(font_menu, MF_STRING, IDM_FONT_FAMILY_COURIER, windows::core::w!("Courier New"))?;
    AppendMenuW(font_menu, MF_STRING, IDM_FONT_FAMILY_LUCIDA, windows::core::w!("Lucida Console"))?;
    AppendMenuW(menu, MF_POPUP, font_menu.0 as usize, windows::core::w!("&Font"))?;

    // Help
    let help_menu = CreatePopupMenu()?;
    AppendMenuW(help_menu, MF_STRING, IDM_HELP_SHORTCUTS, windows::core::w!("Keyboard Shortcuts && Script Usage..."))?;
    AppendMenuW(help_menu, MF_STRING, IDM_HELP_ABOUT, windows::core::w!("About WinRuSSH..."))?;
    AppendMenuW(menu, MF_POPUP, help_menu.0 as usize, windows::core::w!("&Help"))?;

    Ok(menu)
}
