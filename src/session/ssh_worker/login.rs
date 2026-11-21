use crossbeam_channel::Receiver;

use super::SessionCommand;
use crate::session::sink::UiSink;
use crate::terminal::buffer::is_wide_char;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LoginAbort {
    Disconnected,
    Interrupted,
}

/// Terminal-side I/O used during login. `rx == None` means non-interactive
/// (e.g. Lua): any prompt that needs user input aborts the login.
#[derive(Clone, Copy)]
pub struct LoginIo<'a> {
    pub sink: &'a UiSink,
    pub rx: Option<&'a Receiver<SessionCommand>>,
}

/// Reads a single line of user input from the terminal before the SSH shell channel opens.
pub(super) fn read_terminal_line(io: LoginIo<'_>, echo: bool) -> Result<String, LoginAbort> {
    let Some(rx) = io.rx else {
        return Err(LoginAbort::Disconnected);
    };
    let sink = io.sink;
    let mut line = String::new();
    while let Ok(cmd) = rx.recv() {
        match cmd {
            SessionCommand::Disconnect => return Err(LoginAbort::Disconnected),
            SessionCommand::Resize { .. } => {}
            SessionCommand::Input(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                for ch in text.chars() {
                    match ch {
                        '\x03' => {
                            sink.write("^C\r\n");
                            return Err(LoginAbort::Interrupted);
                        }
                        '\r' | '\n' => {
                            sink.write("\r\n");
                            return Ok(line);
                        }
                        '\x08' | '\x7f' => {
                            if let Some(removed) = line.pop() {
                                if echo {
                                    if is_wide_char(removed) {
                                        sink.write("\x08\x08  \x08\x08");
                                    } else {
                                        sink.write("\x08 \x08");
                                    }
                                }
                            }
                        }
                        c if !c.is_control() => {
                            line.push(c);
                            if echo {
                                let mut buf = [0u8; 4];
                                sink.write(c.encode_utf8(&mut buf));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    Err(LoginAbort::Disconnected)
}

pub(super) fn print_banner(sink: &UiSink, banner: &str) {
    if banner.is_empty() {
        return;
    }
    sink.log("Pre-authentication banner message from server:");
    let trimmed = banner.trim_end_matches(['\r', '\n']);
    for line in trimmed.split('\n') {
        let clean = line.trim_end_matches('\r');
        sink.log(&format!("| {}", clean));
    }
    sink.log("End of banner message from server");
}
