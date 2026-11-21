use winrussh::session::{SessionCommand, SshConfig, SshAuth};

#[test]
fn test_session_command_channel() {
    let (tx, rx) = crossbeam_channel::unbounded();
    tx.send(SessionCommand::Input(b"ls\n".to_vec())).unwrap();
    if let Ok(SessionCommand::Input(data)) = rx.recv() {
        assert_eq!(data, b"ls\n");
    } else {
        panic!("Failed to receive SessionCommand::Input");
    }
}

#[test]
fn test_ssh_config_creation() {
    let cfg = SshConfig {
        host: "127.0.0.1".to_string(),
        port: 22,
        username: "testuser".to_string(),
        auth: SshAuth::Password("secret".to_string()),
    };
    assert_eq!(cfg.host, "127.0.0.1");
    assert_eq!(cfg.port, 22);
}
