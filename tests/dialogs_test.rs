use winrussh::ui::canvas::FontConfig;
use winrussh::ui::dialogs::QuickConnectConfig;
use winrussh::ui::{get_config_path, upsert_ini_section};

#[test]
fn test_quick_connect_config_default() {
    let cfg = QuickConnectConfig::default();
    assert_eq!(cfg.host, "127.0.0.1");
    assert_eq!(cfg.port, 22);
    assert_eq!(cfg.username, "root");
    assert_eq!(cfg.password, "");
}

#[test]
fn test_quick_connect_config_parse() {
    let ini = r#"
# Comment
[QuickConnect]
host = test.rebex.net
port = 2222
username = demo_user
password = secret_pass123
"#;
    let cfg = QuickConnectConfig::parse(ini);
    assert_eq!(cfg.host, "test.rebex.net");
    assert_eq!(cfg.port, 2222);
    assert_eq!(cfg.username, "demo_user");
    assert_eq!(cfg.password, "secret_pass123");
}

#[test]
fn test_quick_connect_config_roundtrip() {
    let original = QuickConnectConfig {
        host: "myserver.internal".to_string(),
        port: 8022,
        username: "admin".to_string(),
        password: "p@ssw0rd!#$".to_string(),
        log_enabled: true,
        log_path: r"C:\logs\ssh_session.log".to_string(),
    };
    let serialized = original.to_ini();
    let parsed = QuickConnectConfig::parse(&serialized);
    assert_eq!(parsed, original);
}

#[test]
fn test_get_config_path_in_exe_dir() {
    let path = get_config_path();
    assert_eq!(path.file_name().and_then(|n| n.to_str()), Some("winrussh.ini"));
    let mut expected_dir = std::env::current_exe().unwrap();
    expected_dir.pop();
    assert_eq!(path.parent(), Some(expected_dir.as_path()));
}

#[test]
fn test_upsert_ini_section_preserves_other_sections() {
    let qc1 = QuickConnectConfig {
        host: "10.0.0.1".to_string(),
        port: 22,
        username: "alice".to_string(),
        password: "pw1".to_string(),
        log_enabled: false,
        log_path: String::new(),
    };
    let font1 = FontConfig {
        font_name: "Cascadia Mono".to_string(),
        font_size: 16,
    };

    let ini = upsert_ini_section("", "QuickConnect", &qc1.to_ini());
    let ini = upsert_ini_section(&ini, "Font", &font1.to_ini());

    let qc_parsed = QuickConnectConfig::parse(&ini);
    let font_parsed = FontConfig::parse(&ini);
    assert_eq!(qc_parsed, qc1);
    assert_eq!(font_parsed, font1);

    let font2 = FontConfig {
        font_name: "Consolas".to_string(),
        font_size: 20,
    };
    let ini2 = upsert_ini_section(&ini, "Font", &font2.to_ini());
    assert_eq!(QuickConnectConfig::parse(&ini2), qc1);
    assert_eq!(FontConfig::parse(&ini2), font2);
}
