use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;
use crossbeam_channel::{Receiver, Sender};
use parking_lot::Mutex;
use ssh2::{Channel, Session};

use crate::terminal::buffer::TerminalBuffer;
use super::sink::{SendHwnd, UiSink};

#[derive(Clone, Debug)]
pub enum SshAuth {
    Password(String),
    KeyFile {
        path: String,
        passphrase: Option<String>,
    },
}

#[derive(Clone, Debug)]
pub struct SshConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: SshAuth,
}

#[derive(Clone, Debug)]
pub enum SessionCommand {
    Input(Vec<u8>),
    Resize { cols: u32, rows: u32 },
    Disconnect,
}

pub fn spawn_ssh_worker(
    config: SshConfig,
    buffer: Arc<Mutex<TerminalBuffer>>,
    hwnd_notify: Option<SendHwnd>,
    tab_id: usize,
) -> (Sender<SessionCommand>, std::thread::JoinHandle<()>) {
    let (tx, rx) = crossbeam_channel::unbounded();
    let sink = UiSink::new(Some(buffer), hwnd_notify, tab_id);

    let handle = std::thread::spawn(move || {
        run_worker_thread(config, sink, rx);
    });

    (tx, handle)
}

mod auth;
mod known_hosts;
mod login;
#[cfg(test)]
mod tests;

use auth::authenticate_session_with;
use known_hosts::{default_known_hosts_path, verify_host_key_with};
pub use login::LoginIo;

/// Shared by the interactive tab worker and the Lua `ssh.connect`.
/// Connects, verifies the host key against `known_hosts`, authenticates, opens a PTY shell
/// and switches the session to non-blocking.
pub fn connect_ssh(config: &SshConfig, io: LoginIo<'_>) -> Result<(Session, Channel), String> {
    connect_ssh_with_options(config, io, false)
}

/// Variant of `connect_ssh` allowing non-interactive callers (e.g. Lua `ssh.connect`)
/// to opt into `accept_new_host_key` (trust-on-first-use without prompting).
pub fn connect_ssh_with_options(
    config: &SshConfig,
    io: LoginIo<'_>,
    accept_new_host_key: bool,
) -> Result<(Session, Channel), String> {
    let sink = io.sink;
    let fail = |msg: String| -> String {
        sink.error(&msg);
        msg
    };

    let addr = format!("{}:{}", config.host, config.port);
    let sock_addr = addr
        .to_socket_addrs()
        .map_err(|e| fail(format!("Address resolution error: {}", e)))?
        .next()
        .ok_or_else(|| fail(format!("Unable to resolve address: {}", addr)))?;

    let tcp = TcpStream::connect_timeout(&sock_addr, Duration::from_secs(10))
        .map_err(|e| fail(format!("TCP connection failed: {}", e)))?;

    let mut session = Session::new()
        .map_err(|e| fail(format!("SSH session initialization failed: {}", e)))?;
    session.set_tcp_stream(tcp);
    session
        .handshake()
        .map_err(|e| fail(format!("SSH Handshake failed: {}", e)))?;

    let known_hosts_path = default_known_hosts_path();
    verify_host_key_with(
        &session,
        &config.host,
        config.port,
        io,
        &known_hosts_path,
        accept_new_host_key,
    )?;

    if !authenticate_session_with(&session, config, io) {
        return Err("Authentication failed".to_string());
    }

    let mut channel = session
        .channel_session()
        .map_err(|e| fail(format!("Failed to open channel: {}", e)))?;

    let (cols, rows) = sink.size();
    if let Err(e) = channel.request_pty("xterm-256color", None, Some((cols, rows, 0, 0))) {
        sink.warn(&format!("PTY request failed: {}", e));
    }
    channel
        .shell()
        .map_err(|e| fail(format!("Shell start failed: {}", e)))?;

    session.set_blocking(false);
    Ok((session, channel))
}

fn run_worker_thread(config: SshConfig, sink: UiSink, rx: Receiver<SessionCommand>) {
    let io = LoginIo { sink: &sink, rx: Some(&rx) };
    let (_session, mut channel) = match connect_ssh(&config, io) {
        Ok(v) => v,
        Err(_) => {
            sink.notify_status(false);
            return;
        }
    };

    sink.notify_status(true);

    let mut read_buf = [0u8; 4096];
    loop {
        // 1. Process outgoing commands from UI
        while let Ok(cmd) = rx.try_recv() {
            match cmd {
                SessionCommand::Input(bytes) => {
                    let _ = channel.write_all(&bytes);
                    let _ = channel.flush();
                }
                SessionCommand::Resize { cols, rows } => {
                    let _ = channel.request_pty_size(cols, rows, None, None);
                }
                SessionCommand::Disconnect => {
                    let _ = channel.close();
                    sink.log("[INFO] Disconnected by user.");
                    return;
                }
            }
        }

        // 2. Read incoming data from SSH channel
        match channel.read(&mut read_buf) {
            Ok(0) => {
                if channel.eof() {
                    sink.log("\r\n[INFO] Remote connection closed.");
                    break;
                }
                std::thread::sleep(Duration::from_millis(15));
            }
            Ok(n) => sink.write_bytes(&read_buf[..n]),
            Err(e) => {
                if e.kind() == std::io::ErrorKind::WouldBlock {
                    std::thread::sleep(Duration::from_millis(15));
                } else {
                    sink.log(&format!("\r\n\x1b[31m[ERROR] Channel read error: {}\x1b[0m", e));
                    break;
                }
            }
        }
    }

    sink.notify_status(false);
}
