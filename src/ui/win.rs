use windows::core::PCWSTR;
use windows::Win32::Foundation::{HINSTANCE, HWND};
use windows::Win32::Graphics::Gdi::{
    CreateFontW, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH, FW_NORMAL,
    HFONT, OUT_DEFAULT_PRECIS,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, HMENU, WINDOW_EX_STYLE, WINDOW_STYLE, WS_CHILD, WS_VISIBLE,
};

/// Unwraps a `Result` inside a window procedure's `WM_CREATE`; on failure returns -1,
/// which makes Windows abort window creation instead of panicking (`panic = "abort"`).
#[macro_export]
macro_rules! ok_or_abort {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(_) => return ::windows::Win32::Foundation::LRESULT(-1),
        }
    };
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Segoe UI, ClearType. `height` is the logical height (negative = character height).
pub unsafe fn create_ui_font(height: i32) -> HFONT {
    CreateFontW(
        height,
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
        DEFAULT_PITCH.0 as u32,
        windows::core::w!("Segoe UI"),
    )
}

/// Creates a visible child control. `id` becomes the control ID (0 = none).
pub unsafe fn create_child(
    parent: HWND,
    instance: HINSTANCE,
    class: PCWSTR,
    text: PCWSTR,
    style: WINDOW_STYLE,
    (x, y, w, h): (i32, i32, i32, i32),
    id: usize,
) -> windows::core::Result<HWND> {
    CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        class,
        text,
        WS_CHILD | WS_VISIBLE | style,
        x,
        y,
        w,
        h,
        Some(parent),
        Some(HMENU(id as _)),
        Some(instance),
        None,
    )
}
