use winrussh::tabs::{TabManager, TabStatus};

#[test]
fn test_tab_status_dot_color() {
    assert_eq!(TabStatus::Idle.dot_color(), None);
    assert!(TabStatus::Connecting.dot_color().is_some());
    assert!(TabStatus::RunningLua.dot_color().is_some());
    assert!(TabStatus::Connected.dot_color().is_some());
    assert!(TabStatus::Disconnected.dot_color().is_some());
}

#[test]
fn test_tab_lifecycle() {
    let mut manager = TabManager::new_headless();
    let id0 = manager.add_tab("Tab 0");
    let id1 = manager.add_tab("Tab 1");
    assert_eq!(id0, 0);
    assert_eq!(id1, 1);
    assert_eq!(manager.tab_count(), 2);
    assert_eq!(manager.active_index(), 0);

    manager.switch_to(1);
    assert_eq!(manager.active_index(), 1);

    manager.close_tab(0);
    assert_eq!(manager.tab_count(), 1);
    assert_eq!(manager.active_index(), 0);
    assert_eq!(manager.active_tab().unwrap().title, "Tab 1");
}

#[test]
fn test_tab_independent_logging_and_close_stops_log() {
    let mut path = std::env::temp_dir();
    path.push(format!("winrussh_tab_log_{}.log", std::process::id()));

    let mut manager = TabManager::new_headless();
    manager.add_tab("Tab 0");
    manager.add_tab("Tab 1");

    let tab0_buf = manager.get_tab(0).unwrap().buffer.clone();
    let tab1_buf = manager.get_tab(1).unwrap().buffer.clone();

    tab0_buf.lock().start_logging(&path).expect("start_logging should succeed");
    tab0_buf.lock().write_bytes(b"tab0 line\r\n");
    tab1_buf.lock().write_bytes(b"tab1 line\r\n");

    assert!(tab0_buf.lock().is_logging());
    assert!(!tab1_buf.lock().is_logging());

    manager.close_tab(0);
    assert!(!tab0_buf.lock().is_logging());

    let recorded = std::fs::read(&path).expect("log file should exist");
    assert_eq!(recorded, b"tab0 line\r\n");

    let _ = std::fs::remove_file(&path);
}
