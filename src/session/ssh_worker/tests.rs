#![allow(unused_imports)]
use super::*;
use super::auth::*;
use super::known_hosts::*;
use super::login::*;
use crate::terminal::buffer::{is_wide_char, TerminalBuffer};
use ssh2::{CheckResult, HashType, HostKeyType, KnownHostFileKind, KnownHostKeyFormat};
use std::path::{Path, PathBuf};
use crate::session::sink::UiSink;

mod host_key_tests {
    use super::*;
    use std::cell::Cell;

    fn make_test_sink() -> (UiSink, Arc<Mutex<TerminalBuffer>>) {
        let buffer = Arc::new(Mutex::new(TerminalBuffer::new(80, 24, 100)));
        let sink = UiSink::new(Some(buffer.clone()), None, 0);
        (sink, buffer)
    }

    struct MockHostKeyBackend {
        result: CheckResult,
        added: Cell<bool>,
    }

    impl MockHostKeyBackend {
        fn new(result: CheckResult) -> Self {
            Self {
                result,
                added: Cell::new(false),
            }
        }
    }

    impl HostKeyBackend for MockHostKeyBackend {
        fn server_host_key(&self) -> Option<(Vec<u8>, HostKeyType)> {
            Some((vec![1, 2, 3, 4], HostKeyType::Ed25519))
        }

        fn server_host_key_sha256(&self) -> Option<Vec<u8>> {
            Some(vec![0u8; 32])
        }

        fn check_known_hosts(
            &self,
            _path: &Path,
            _host: &str,
            _port: u16,
            _key: &[u8],
        ) -> Result<CheckResult, String> {
            Ok(self.result)
        }

        fn add_known_host(
            &self,
            _path: &Path,
            _host: &str,
            _port: u16,
            _key: &[u8],
            _ty: HostKeyType,
        ) -> Result<(), String> {
            self.added.set(true);
            Ok(())
        }
    }

    #[test]
    fn test_host_key_helpers() {
        assert_eq!(known_hosts_entry_name("10.0.0.1", 22), "10.0.0.1");
        assert_eq!(known_hosts_entry_name("10.0.0.1", 2222), "[10.0.0.1]:2222");
        assert_eq!(
            format_sha256_fingerprint(&[0u8; 32]),
            "SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
        );
    }

    #[test]
    fn test_verify_host_key_match_and_mismatch() {
        let (sink, _buffer) = make_test_sink();
        let path = Path::new("known_hosts");

        let ok_backend = MockHostKeyBackend::new(CheckResult::Match);
        assert!(verify_host_key_with(
            &ok_backend,
            "10.0.0.1",
            22,
            LoginIo { sink: &sink, rx: None },
            path,
            false
        )
        .is_ok());
        assert!(!ok_backend.added.get());

        let bad_backend = MockHostKeyBackend::new(CheckResult::Mismatch);
        let err = verify_host_key_with(
            &bad_backend,
            "10.0.0.1",
            22,
            LoginIo { sink: &sink, rx: None },
            path,
            true,
        )
        .unwrap_err();
        assert!(err.contains("REMOTE HOST IDENTIFICATION HAS CHANGED"));
        assert!(!bad_backend.added.get());
    }

    #[test]
    fn test_verify_host_key_not_found_interactive_yes_and_no() {
        let path = Path::new("known_hosts");

        let (sink, _buf) = make_test_sink();
        let (tx, rx) = crossbeam_channel::unbounded();
        tx.send(SessionCommand::Input(b"yes\r".to_vec())).unwrap();
        let backend_yes = MockHostKeyBackend::new(CheckResult::NotFound);
        assert!(verify_host_key_with(
            &backend_yes,
            "10.0.0.1",
            22,
            LoginIo { sink: &sink, rx: Some(&rx) },
            path,
            false,
        )
        .is_ok());
        assert!(backend_yes.added.get());

        let (sink, _buf) = make_test_sink();
        let (tx, rx) = crossbeam_channel::unbounded();
        tx.send(SessionCommand::Input(b"no\r".to_vec())).unwrap();
        let backend_no = MockHostKeyBackend::new(CheckResult::NotFound);
        let err = verify_host_key_with(
            &backend_no,
            "10.0.0.1",
            22,
            LoginIo { sink: &sink, rx: Some(&rx) },
            path,
            false,
        )
        .unwrap_err();
        assert!(err.contains("user declined"));
        assert!(!backend_no.added.get());
    }

    #[test]
    fn test_verify_host_key_not_found_non_interactive() {
        let (sink, _buf) = make_test_sink();
        let path = Path::new("known_hosts");

        let backend_reject = MockHostKeyBackend::new(CheckResult::NotFound);
        let err = verify_host_key_with(
            &backend_reject,
            "10.0.0.1",
            22,
            LoginIo { sink: &sink, rx: None },
            path,
            false,
        )
        .unwrap_err();
        assert!(err.contains("not in known_hosts"));
        assert!(!backend_reject.added.get());

        let backend_accept = MockHostKeyBackend::new(CheckResult::NotFound);
        assert!(verify_host_key_with(
            &backend_accept,
            "10.0.0.1",
            22,
            LoginIo { sink: &sink, rx: None },
            path,
            true,
        )
        .is_ok());
        assert!(backend_accept.added.get());
    }
}

mod tests {
    use super::*;

    fn buffer_line_text(buf: &TerminalBuffer, row: usize) -> String {
        let lines = buf.get_visible_lines(0);
        lines[row]
            .iter()
            .filter(|c| !c.is_wide_continuation)
            .map(|c| c.ch)
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    #[test]
    fn test_read_terminal_line_echo_and_backspace() {
        let buffer = Arc::new(Mutex::new(TerminalBuffer::new(80, 24, 100)));
        let sink = UiSink::new(Some(buffer.clone()), None, 0);
        let (tx, rx) = crossbeam_channel::unbounded();

        sink.write("login as: ");
        tx.send(SessionCommand::Input(b"demx".to_vec())).unwrap();
        tx.send(SessionCommand::Input(vec![0x08])).unwrap();
        tx.send(SessionCommand::Input(b"o\r".to_vec())).unwrap();

        let res = read_terminal_line(LoginIo { sink: &sink, rx: Some(&rx) }, true);
        assert_eq!(res, Ok("demo".to_string()));
        let b = buffer.lock();
        assert_eq!(buffer_line_text(&b, 0), "login as: demo");
    }

    #[test]
    fn test_read_terminal_line_no_echo_password() {
        let buffer = Arc::new(Mutex::new(TerminalBuffer::new(80, 24, 100)));
        let sink = UiSink::new(Some(buffer.clone()), None, 0);
        let (tx, rx) = crossbeam_channel::unbounded();

        sink.write("| Password: ");
        tx.send(SessionCommand::Input(b"passw".to_vec())).unwrap();
        tx.send(SessionCommand::Input(vec![0x7f])).unwrap();
        tx.send(SessionCommand::Input(b"word\r".to_vec())).unwrap();

        let res = read_terminal_line(LoginIo { sink: &sink, rx: Some(&rx) }, false);
        assert_eq!(res, Ok("password".to_string()));
        let b = buffer.lock();
        assert_eq!(buffer_line_text(&b, 0), "| Password:");
    }

    #[test]
    fn test_read_terminal_line_ctrl_c_and_disconnect() {
        let buffer = Arc::new(Mutex::new(TerminalBuffer::new(80, 24, 100)));
        let sink = UiSink::new(Some(buffer.clone()), None, 0);
        let (tx, rx) = crossbeam_channel::unbounded();

        tx.send(SessionCommand::Input(b"ab".to_vec())).unwrap();
        tx.send(SessionCommand::Input(vec![0x03])).unwrap();
        assert_eq!(
            read_terminal_line(LoginIo { sink: &sink, rx: Some(&rx) }, true),
            Err(LoginAbort::Interrupted)
        );

        tx.send(SessionCommand::Disconnect).unwrap();
        assert_eq!(
            read_terminal_line(LoginIo { sink: &sink, rx: Some(&rx) }, true),
            Err(LoginAbort::Disconnected)
        );
    }

    #[test]
    fn test_print_banner_putty_format() {
        let buffer = Arc::new(Mutex::new(TerminalBuffer::new(100, 24, 100)));
        let sink = UiSink::new(Some(buffer.clone()), None, 0);
        print_banner(&sink, "Welcome to test.rebex.net! See https://test.rebex.net/ for more information.\r\n",
        );

        let b = buffer.lock();
        assert_eq!(
            buffer_line_text(&b, 0),
            "Pre-authentication banner message from server:"
        );
        assert_eq!(
            buffer_line_text(&b, 1),
            "| Welcome to test.rebex.net! See https://test.rebex.net/ for more information."
        );
        assert_eq!(buffer_line_text(&b, 2), "End of banner message from server");
    }

    struct MockAuthBackend {
        methods: &'static str,
        kbi_prompt_limit: usize,
        kbi_calls: std::cell::Cell<usize>,
        expected_password: &'static str,
        authed: std::cell::Cell<bool>,
    }

    impl MockAuthBackend {
        fn new(methods: &'static str, kbi_prompt_limit: usize, expected_password: &'static str) -> Self {
            Self {
                methods,
                kbi_prompt_limit,
                kbi_calls: std::cell::Cell::new(0),
                expected_password,
                authed: std::cell::Cell::new(false),
            }
        }
    }

    impl AuthBackend for MockAuthBackend {
        fn auth_methods(&self, _username: &str) -> Result<&str, ssh2::Error> {
            Ok(self.methods)
        }

        fn banner(&self) -> Option<&str> {
            None
        }

        fn authenticated(&self) -> bool {
            self.authed.get()
        }

        fn userauth_pubkey_file(
            &self,
            _username: &str,
            _pubkey: Option<&Path>,
            _privatekey: &Path,
            _passphrase: Option<&str>,
        ) -> Result<(), ssh2::Error> {
            Err(ssh2::Error::new(ssh2::ErrorCode::Session(-18), "auth failed"))
        }

        fn userauth_password(&self, _username: &str, password: &str) -> Result<(), ssh2::Error> {
            if password == self.expected_password {
                self.authed.set(true);
                Ok(())
            } else {
                Err(ssh2::Error::new(ssh2::ErrorCode::Session(-18), "auth failed"))
            }
        }

        fn userauth_keyboard_interactive(
            &self,
            username: &str,
            prompter: &mut TerminalKbiPrompter<'_>,
        ) -> Result<(), ssh2::Error> {
            use ssh2::KeyboardInteractivePrompt;
            let call_idx = self.kbi_calls.get();
            self.kbi_calls.set(call_idx + 1);

            if call_idx < self.kbi_prompt_limit {
                let prompts = [ssh2::Prompt {
                    text: "Password: ".into(),
                    echo: false,
                }];
                let responses = prompter.prompt(username, "", &prompts);
                if responses.first().map(|s| s.as_str()) == Some(self.expected_password) {
                    self.authed.set(true);
                    return Ok(());
                }
            }
            Err(ssh2::Error::new(ssh2::ErrorCode::Session(-18), "auth failed"))
        }
    }

    #[test]
    fn test_kbi_stops_when_server_refuses_further_prompts() {
        let buffer = Arc::new(Mutex::new(TerminalBuffer::new(100, 24, 100)));
        let sink = UiSink::new(Some(buffer.clone()), None, 0);
        let (tx, rx) = crossbeam_channel::unbounded();
        let config = SshConfig {
            host: "example.com".to_string(),
            port: 22,
            username: "demp".to_string(),
            auth: SshAuth::Password(String::new()),
        };
        let backend = MockAuthBackend::new("keyboard-interactive", 1, "secret");

        tx.send(SessionCommand::Input(b"wrong\r".to_vec())).unwrap();

        let ok = authenticate_session_with(&backend, &config, LoginIo { sink: &sink, rx: Some(&rx) });
        assert!(!ok);
        assert_eq!(backend.kbi_calls.get(), 2);

        let b = buffer.lock();
        let all_lines: Vec<String> = (0..12)
            .map(|r| buffer_line_text(&b, r))
            .filter(|s| !s.is_empty())
            .collect();
        let denied_count = all_lines.iter().filter(|l| *l == "Access denied").count();
        assert_eq!(denied_count, 1, "lines: {:?}", all_lines);
        assert!(
            all_lines
                .iter()
                .any(|l| l.contains("Server refused further authentication attempts.")),
            "lines: {:?}",
            all_lines
        );
    }

    #[test]
    fn test_kbi_falls_back_to_password_when_not_prompted() {
        let buffer = Arc::new(Mutex::new(TerminalBuffer::new(100, 24, 100)));
        let sink = UiSink::new(Some(buffer.clone()), None, 0);
        let (tx, rx) = crossbeam_channel::unbounded();
        let config = SshConfig {
            host: "example.com".to_string(),
            port: 22,
            username: "demo".to_string(),
            auth: SshAuth::Password(String::new()),
        };
        let backend = MockAuthBackend::new("keyboard-interactive,password", 1, "secret");

        tx.send(SessionCommand::Input(b"wrong\r".to_vec())).unwrap();
        tx.send(SessionCommand::Input(b"secret\r".to_vec())).unwrap();

        let ok = authenticate_session_with(&backend, &config, LoginIo { sink: &sink, rx: Some(&rx) });
        assert!(ok);
        assert_eq!(backend.kbi_calls.get(), 2);

        let b = buffer.lock();
        let all_lines: Vec<String> = (0..12)
            .map(|r| buffer_line_text(&b, r))
            .filter(|s| !s.is_empty())
            .collect();
        let denied_count = all_lines.iter().filter(|l| *l == "Access denied").count();
        assert_eq!(denied_count, 1, "lines: {:?}", all_lines);
        assert!(
            all_lines
                .iter()
                .any(|l| l.contains("demo@example.com's password:")),
            "lines: {:?}",
            all_lines
        );
    }
}

