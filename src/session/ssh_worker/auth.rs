use std::path::Path;

use ssh2::Session;

use super::login::*;
use super::{SshAuth, SshConfig};

pub(super) struct TerminalKbiPrompter<'a> {
    io: LoginIo<'a>,
    pre_password: Option<String>,
    aborted: bool,
    prompted: bool,
}

impl<'a> TerminalKbiPrompter<'a> {
    fn new(io: LoginIo<'a>, pre_password: Option<String>) -> Self {
        Self { io, pre_password, aborted: false, prompted: false }
    }
}

impl<'a> ssh2::KeyboardInteractivePrompt for TerminalKbiPrompter<'a> {
    fn prompt<'b>(
        &mut self,
        _username: &str,
        instructions: &str,
        prompts: &[ssh2::Prompt<'b>],
    ) -> Vec<String> {
        if let Some(pw) = self.pre_password.take() {
            if prompts.len() == 1 && !prompts[0].echo {
                self.prompted = true;
                return vec![pw];
            }
        }

        if prompts.is_empty() && instructions.is_empty() {
            return Vec::new();
        }

        if !prompts.is_empty() {
            self.prompted = true;
        }

        let sink = self.io.sink;
        sink.log("Keyboard-interactive authentication prompts from server:");
        if !instructions.is_empty() {
            for line in instructions.trim_end_matches(['\r', '\n']).split('\n') {
                sink.log(&format!("| {}", line.trim_end_matches('\r')));
            }
        }

        let mut responses = Vec::with_capacity(prompts.len());
        for p in prompts {
            sink.write(&format!("| {}", p.text));
            match read_terminal_line(self.io, p.echo) {
                Ok(ans) => responses.push(ans),
                Err(_) => {
                    self.aborted = true;
                    responses.resize(prompts.len(), String::new());
                    return responses;
                }
            }
        }

        sink.log("End of keyboard-interactive prompts from server");
        responses
    }
}

pub(super) fn is_auth_failed_err(err: &ssh2::Error) -> bool {
    matches!(err.code(), ssh2::ErrorCode::Session(-18 | -19))
}

pub(super) trait AuthBackend {
    fn auth_methods(&self, username: &str) -> Result<&str, ssh2::Error>;
    fn banner(&self) -> Option<&str>;
    fn authenticated(&self) -> bool;
    fn userauth_pubkey_file(
        &self,
        username: &str,
        pubkey: Option<&Path>,
        privatekey: &Path,
        passphrase: Option<&str>,
    ) -> Result<(), ssh2::Error>;
    fn userauth_password(&self, username: &str, password: &str) -> Result<(), ssh2::Error>;
    fn userauth_keyboard_interactive(
        &self,
        username: &str,
        prompter: &mut TerminalKbiPrompter<'_>,
    ) -> Result<(), ssh2::Error>;
}

impl AuthBackend for Session {
    fn auth_methods(&self, username: &str) -> Result<&str, ssh2::Error> {
        Session::auth_methods(self, username)
    }

    fn banner(&self) -> Option<&str> {
        Session::banner(self)
    }

    fn authenticated(&self) -> bool {
        Session::authenticated(self)
    }

    fn userauth_pubkey_file(
        &self,
        username: &str,
        pubkey: Option<&Path>,
        privatekey: &Path,
        passphrase: Option<&str>,
    ) -> Result<(), ssh2::Error> {
        Session::userauth_pubkey_file(self, username, pubkey, privatekey, passphrase)
    }

    fn userauth_password(&self, username: &str, password: &str) -> Result<(), ssh2::Error> {
        Session::userauth_password(self, username, password)
    }

    fn userauth_keyboard_interactive(
        &self,
        username: &str,
        prompter: &mut TerminalKbiPrompter<'_>,
    ) -> Result<(), ssh2::Error> {
        Session::userauth_keyboard_interactive(self, username, prompter)
    }
}

pub(super) fn authenticate_session_with(
    session: &dyn AuthBackend,
    config: &SshConfig,
    io: LoginIo<'_>,
) -> bool {
    let sink = io.sink;
    let username = if config.username.trim().is_empty() {
        loop {
            sink.write("login as: ");
            match read_terminal_line(io, true) {
                Ok(u) => {
                    let trimmed = u.trim().to_string();
                    if !trimmed.is_empty() {
                        break trimmed;
                    }
                }
                Err(_) => return false,
            }
        }
    } else {
        let u = config.username.trim().to_string();
        sink.log(&format!("Using username \"{}\".", u));
        u
    };

    let methods_str = session.auth_methods(&username).unwrap_or("").to_string();
    let mut banner_shown = false;
    if let Some(banner) = session.banner() {
        print_banner(sink, banner);
        banner_shown = true;
    }

    if session.authenticated() {
        return true;
    }

    let check_banner = |shown: &mut bool| {
        if !*shown {
            if let Some(b) = session.banner() {
                print_banner(sink, b);
                *shown = true;
            }
        }
    };

    let has_method = |name: &str| methods_str.split(',').any(|m| m.trim() == name);

    match &config.auth {
        SshAuth::KeyFile { path, passphrase } => {
            if let Err(e) = session.userauth_pubkey_file(
                &username,
                None,
                Path::new(path),
                passphrase.as_deref(),
            ) {
                check_banner(&mut banner_shown);
                sink.log(&format!("Server refused our key ({})", e));
            } else {
                check_banner(&mut banner_shown);
                if session.authenticated() {
                    return true;
                }
            }
        }
        SshAuth::Password(pass) if !pass.is_empty() => {
            if has_method("password") || methods_str.is_empty() {
                let _ = session.userauth_password(&username, pass);
                check_banner(&mut banner_shown);
                if session.authenticated() {
                    return true;
                }
            }
            if has_method("keyboard-interactive") {
                let mut prompter = TerminalKbiPrompter::new(io, Some(pass.clone()));
                let _ = session.userauth_keyboard_interactive(&username, &mut prompter);
                check_banner(&mut banner_shown);
                if prompter.aborted {
                    return false;
                }
                if session.authenticated() {
                    return true;
                }
            }
            sink.log("Access denied");
        }
        _ => {}
    }

    // Interactive authentication loop (PuTTY style)
    const MAX_ATTEMPTS: usize = 5;
    let mut kbi_available = has_method("keyboard-interactive");
    let password_available = has_method("password") || methods_str.is_empty();

    for _ in 0..MAX_ATTEMPTS {
        if kbi_available {
            let mut prompter = TerminalKbiPrompter::new(io, None);
            let res = session.userauth_keyboard_interactive(&username, &mut prompter);
            check_banner(&mut banner_shown);
            if prompter.aborted {
                return false;
            }
            if session.authenticated() {
                return true;
            }
            if let Err(e) = res {
                if !is_auth_failed_err(&e) {
                    sink.error(&format!("Authentication error: {}", e));
                    return false;
                }
            }
            if prompter.prompted {
                sink.log("Access denied");
                continue;
            }
            // Server rejected keyboard-interactive without prompting; avoid retrying it.
            kbi_available = false;
        }

        if password_available {
            sink.write(&format!("{}@{}'s password: ", username, config.host));
            let pw = match read_terminal_line(io, false) {
                Ok(p) => p,
                Err(_) => return false,
            };
            let res = session.userauth_password(&username, &pw);
            check_banner(&mut banner_shown);
            if session.authenticated() {
                return true;
            }
            if let Err(e) = res {
                if !is_auth_failed_err(&e) {
                    sink.error(&format!("Authentication error: {}", e));
                    return false;
                }
            }
            sink.log("Access denied");
        } else {
            if has_method("keyboard-interactive") {
                sink.error("Server refused further authentication attempts.");
            } else {
                sink.error(&format!("No supported authentication methods ({}).", methods_str));
            }
            return false;
        }
    }

    sink.error("Too many authentication failures.");
    false
}
