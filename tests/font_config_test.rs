use winrussh::ui::canvas::FontConfig;

#[test]
fn test_font_config_default() {
    let cfg = FontConfig::default();
    assert_eq!(cfg.font_name, "Consolas");
    assert_eq!(cfg.font_size, 16);
}

#[test]
fn test_font_config_parse_and_clamp() {
    let ini = r#"
[Font]
font_name = Cascadia Mono
font_size = 200
"#;
    let cfg = FontConfig::parse(ini);
    assert_eq!(cfg.font_name, "Cascadia Mono");
    assert_eq!(cfg.font_size, 72); // clamped max
}

#[test]
fn test_font_config_roundtrip() {
    let original = FontConfig {
        font_name: "Courier New".to_string(),
        font_size: 20,
    };
    let serialized = original.to_ini();
    let parsed = FontConfig::parse(&serialized);
    assert_eq!(parsed, original);
}
