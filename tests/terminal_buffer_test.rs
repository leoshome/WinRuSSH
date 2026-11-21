use winrussh::terminal::buffer::TerminalBuffer;

#[test]
fn test_plain_text_write() {
    let mut buf = TerminalBuffer::new(80, 24, 1000);
    buf.write_bytes(b"Hello World\r\nLine 2");
    let lines = buf.get_visible_lines(0);
    let line0: String = lines[0].iter().map(|c| c.ch).collect();
    let line1: String = lines[1].iter().map(|c| c.ch).collect();
    assert!(line0.starts_with("Hello World"));
    assert!(line1.starts_with("Line 2"));
}

#[test]
fn test_ansi_color_parsing() {
    let mut buf = TerminalBuffer::new(80, 24, 1000);
    // Red foreground text
    buf.write_bytes(b"\x1b[31mRed\x1b[0mNormal");
    let lines = buf.get_visible_lines(0);
    assert_eq!(lines[0][0].ch, 'R');
    assert_eq!(lines[0][0].fg_color, 0x0000FF); // Windows COLORREF Red (0x000000FF or RGB(255, 0, 0))
    assert_eq!(lines[0][3].ch, 'N');
    assert_eq!(lines[0][3].fg_color, 0xCCCCCC); // Reset normal foreground
}

#[test]
fn test_cursor_and_clear_line() {
    let mut buf = TerminalBuffer::new(80, 24, 1000);
    buf.write_bytes(b"ABCDE\r\x1b[2KTest");
    let lines = buf.get_visible_lines(0);
    let line0: String = lines[0].iter().map(|c| c.ch).collect();
    assert!(line0.starts_with("Test"));
}

#[test]
fn test_utf8_chinese_characters() {
    let mut buf = TerminalBuffer::new(80, 24, 1000);
    buf.write_bytes("測試中文".as_bytes());
    let lines = buf.get_visible_lines(0);
    assert_eq!(lines[0][0].ch, '測');
    assert_eq!(lines[0][2].ch, '試');
    assert_eq!(lines[0][4].ch, '中');
    assert_eq!(lines[0][6].ch, '文');
}

#[test]
fn test_raw_logging_start_write_stop() {
    let mut path = std::env::temp_dir();
    path.push(format!("winrussh_buf_log_{}.log", std::process::id()));

    let mut buf = TerminalBuffer::new(80, 24, 1000);
    assert!(!buf.is_logging());
    assert!(buf.log_path().is_none());

    buf.write_bytes(b"Before log\r\n");

    buf.start_logging(&path).expect("start_logging should succeed");
    assert!(buf.is_logging());
    assert_eq!(buf.log_path(), Some(path.as_path()));

    let raw_payload = b"\x1b[32mGreen\x1b[0m \xe6\xb8\xac\xe8\xa9\xa6\r\n";
    buf.write_bytes(raw_payload);

    buf.stop_logging();
    assert!(!buf.is_logging());
    assert!(buf.log_path().is_none());

    buf.write_bytes(b"After log\r\n");

    let recorded = std::fs::read(&path).expect("log file should exist");
    assert_eq!(recorded, raw_payload);

    let _ = std::fs::remove_file(&path);
}

#[test]
fn test_resize_preserves_text_after_shrink_and_expand() {
    let mut buf = TerminalBuffer::new(80, 24, 1000);
    buf.write_bytes(b"Quick Start:\r\n  * [F2] or Connect -> Quick Connect\r\n");

    // Simulate narrowing (e.g., transient small width) then restoring original width
    buf.resize(10, 5);
    buf.resize(80, 24);

    let lines = buf.get_visible_lines(0);
    let line0: String = lines[0].iter().map(|c| c.ch).collect();
    let line1: String = lines[1].iter().map(|c| c.ch).collect();
    assert!(line0.starts_with("Quick Start:"));
    assert!(line1.starts_with("  * [F2] or Connect -> Quick Connect"));
}

#[test]
fn test_resize_smaller_trims_trailing_blank_lines_without_phantom_scrollback() {
    let mut buf = TerminalBuffer::new(80, 24, 1000);
    buf.resize(80, 10);
    assert_eq!(buf.total_lines(), 10);
    assert_eq!(buf.cursor_pos(), (0, 0));

    buf.write_bytes(b"The authenticity of host\r\nAre you sure?");
    let lines = buf.get_visible_lines(0);
    let line0: String = lines[0].iter().map(|c| c.ch).collect();
    let line1: String = lines[1].iter().map(|c| c.ch).collect();
    assert!(line0.starts_with("The authenticity of host"));
    assert!(line1.starts_with("Are you sure?"));
    assert_eq!(buf.total_lines(), 10);
}
