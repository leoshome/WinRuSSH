use std::io::{Read, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};
use crossbeam_channel::{Receiver, TryRecvError};
use mlua::prelude::*;
use parking_lot::Mutex;
use ssh2::{Channel, Session};

use crate::session::{connect_ssh_with_options, LoginIo, SshAuth, SshConfig, UiSink};

pub struct SharedSshContext {
    pub session: Option<Session>,
    pub channel: Option<Channel>,
    pub sink: UiSink,
    pub input_rx: Option<Receiver<Vec<u8>>>,
    pub unread_buf: String,
}

impl SharedSshContext {
    pub fn new(sink: UiSink, input_rx: Option<Receiver<Vec<u8>>>) -> Self {
        Self {
            session: None,
            channel: None,
            sink,
            input_rx,
            unread_buf: String::new(),
        }
    }

    /// Forward any pending manual keyboard bytes to the active SSH channel.
    /// Returns `false` if the input channel was disconnected (tab closed).
    pub fn forward_keyboard_input(&mut self) -> bool {
        let Some(ref rx) = self.input_rx else {
            return true;
        };
        loop {
            match rx.try_recv() {
                Ok(bytes) => {
                    if let Some(ref mut ch) = self.channel {
                        let _ = ch.write_all(&bytes);
                        let _ = ch.flush();
                    }
                }
                Err(TryRecvError::Empty) => return true,
                Err(TryRecvError::Disconnected) => return false,
            }
        }
    }
}

pub fn register_ssh_module(
    lua: &Lua,
    context: Arc<Mutex<SharedSshContext>>,
) -> LuaResult<()> {
    let ssh_table = lua.create_table()?;

    // ssh.sleep(ms) - forwards manual keyboard input & output while waiting
    let ctx_sleep = context.clone();
    let sleep_fn = lua.create_function(move |_, ms: u64| {
        let duration = Duration::from_millis(ms);
        let start = Instant::now();
        let mut buf = [0u8; 1024];
        while start.elapsed() < duration {
            {
                let mut ctx = ctx_sleep.lock();
                ctx.forward_keyboard_input();
                if let Some(ref mut ch) = ctx.channel {
                    if let Ok(n) = ch.read(&mut buf) {
                        if n > 0 {
                            let text = String::from_utf8_lossy(&buf[..n]).into_owned();
                            ctx.unread_buf.push_str(&text);
                            ctx.sink.write_bytes(&buf[..n]);
                        }
                    }
                }
            }
            let remaining = duration.saturating_sub(start.elapsed());
            if remaining.is_zero() {
                break;
            }
            std::thread::sleep(remaining.min(Duration::from_millis(15)));
        }
        Ok(())
    })?;
    ssh_table.set("sleep", sleep_fn)?;

    // ssh.connect(opts) -> (ok, err)
    // opts: host, port?, username, password? | key_path? (+ passphrase?)
    // Uses the same connect/auth path as interactive tabs (banner, keyboard-interactive, ...).
    let ctx_connect = context.clone();
    let connect_fn = lua.create_function(move |_, opts: LuaTable| {
        let host: String = opts.get("host")?;
        let port: u16 = opts.get("port").unwrap_or(22);
        let username: String = opts.get("username")?;
        let password: Option<String> = opts.get("password").ok();
        let key_path: Option<String> = opts.get("key_path").ok();
        let passphrase: Option<String> = opts.get("passphrase").ok();
        let accept_new: bool = opts.get("accept_new").unwrap_or(false);

        let auth = match (password, key_path) {
            (Some(p), _) => SshAuth::Password(p),
            (None, Some(path)) => SshAuth::KeyFile { path, passphrase },
            (None, None) => SshAuth::Password(String::new()),
        };
        let config = SshConfig { host, port, username, auth };

        let sink = ctx_connect.lock().sink.clone();
        sink.log(&format!(
            "[Lua] Connecting to {}:{} as {}...",
            config.host, config.port, config.username
        ));

        match connect_ssh_with_options(&config, LoginIo { sink: &sink, rx: None }, accept_new) {
            Ok((session, channel)) => {
                let mut ctx = ctx_connect.lock();
                ctx.session = Some(session);
                ctx.channel = Some(channel);
                sink.log("\x1b[32m[Lua] Connected successfully!\x1b[0m");
                Ok((true, None))
            }
            Err(err) => Ok((false, Some(err))),
        }
    })?;
    ssh_table.set("connect", connect_fn)?;
    // ssh.send(text)
    let ctx_send = context.clone();
    let send_fn = lua.create_function(move |_, text: String| {
        let mut ctx = ctx_send.lock();
        if let Some(ref mut ch) = ctx.channel {
            let _ = ch.write_all(text.as_bytes());
            let _ = ch.flush();
            Ok(true)
        } else {
            Ok(false)
        }
    })?;
    ssh_table.set("send", send_fn)?;

    // ssh.expect(pattern_or_patterns, timeout_ms)
    // Supports single string or table of strings: {"$", "#", ">", "Password:"}
    // Returns: (match_index, captured_text, matched_pattern)
    let ctx_expect = context.clone();
    let expect_fn = lua.create_function(move |_, (pattern_val, timeout_ms): (LuaValue, Option<u64>)| {
        let patterns: Vec<String> = match pattern_val {
            LuaValue::String(s) => vec![s.to_str()?.to_string()],
            LuaValue::Table(tbl) => {
                let mut list = Vec::new();
                for val in tbl.sequence_values::<String>() {
                    list.push(val?);
                }
                list
            }
            _ => return Err(LuaError::runtime("ssh.expect expects a string or array of strings as first argument")),
        };

        let timeout = Duration::from_millis(timeout_ms.unwrap_or(5000));
        let start = Instant::now();
        let mut accumulated = {
            let mut ctx = ctx_expect.lock();
            std::mem::take(&mut ctx.unread_buf)
        };

        // Check if unread buffer already contains any target pattern
        for (idx, pat) in patterns.iter().enumerate() {
            if let Some(pos) = accumulated.find(pat) {
                let captured = accumulated[..pos].to_string();
                let remaining = accumulated[pos + pat.len()..].to_string();
                let mut ctx = ctx_expect.lock();
                ctx.unread_buf = remaining;
                return Ok((Some(idx + 1), captured, Some(pat.clone())));
            }
        }

        let mut buf = [0u8; 1024];
        while start.elapsed() < timeout {
            let mut ctx = ctx_expect.lock();
            ctx.forward_keyboard_input();
            if let Some(ref mut ch) = ctx.channel {
                match ch.read(&mut buf) {
                    Ok(n) if n > 0 => {
                        let text = String::from_utf8_lossy(&buf[..n]);
                        accumulated.push_str(&text);
                        ctx.sink.write_bytes(&buf[..n]);

                        // Check patterns
                        for (idx, pat) in patterns.iter().enumerate() {
                            if let Some(pos) = accumulated.find(pat) {
                                let captured = accumulated[..pos].to_string();
                                let remaining = accumulated[pos + pat.len()..].to_string();
                                ctx.unread_buf = remaining;
                                return Ok((Some(idx + 1), captured, Some(pat.clone())));
                            }
                        }
                    }
                    _ => {}
                }
            }
            drop(ctx);
            std::thread::sleep(Duration::from_millis(15));
        }

        // Timeout reached: retain accumulated bytes in unread_buf so data is preserved
        let mut ctx = ctx_expect.lock();
        let captured = accumulated.clone();
        ctx.unread_buf = accumulated;
        Ok((None, captured, None))
    })?;
    ssh_table.set("expect", expect_fn)?;

    // ssh.read(timeout_ms)
    let ctx_read = context.clone();
    let read_fn = lua.create_function(move |_, timeout_ms: Option<u64>| {
        let timeout = Duration::from_millis(timeout_ms.unwrap_or(500));
        let start = Instant::now();
        let mut accumulated = String::new();
        let mut buf = [0u8; 1024];

        while start.elapsed() < timeout {
            let mut ctx = ctx_read.lock();
            ctx.forward_keyboard_input();
            if let Some(ref mut ch) = ctx.channel {
                if let Ok(n) = ch.read(&mut buf) {
                    if n > 0 {
                        accumulated.push_str(&String::from_utf8_lossy(&buf[..n]));
                        ctx.sink.write_bytes(&buf[..n]);
                    }
                }
            }
            drop(ctx);
            std::thread::sleep(Duration::from_millis(15));
        }

        Ok(accumulated)
    })?;
    ssh_table.set("read", read_fn)?;

    lua.globals().set("ssh", ssh_table)?;
    Ok(())
}

/// After a Lua script finishes, keep the SSH session interactive if still connected.
pub fn run_post_script_interactive(context: &Arc<Mutex<SharedSshContext>>) {
    let (has_channel, sink) = {
        let ctx = context.lock();
        (ctx.channel.is_some(), ctx.sink.clone())
    };
    if !has_channel {
        return;
    }

    let mut read_buf = [0u8; 2048];
    loop {
        let mut ctx = context.lock();
        if !ctx.forward_keyboard_input() {
            break;
        }
        if let Some(ref mut ch) = ctx.channel {
            match ch.read(&mut read_buf) {
                Ok(n) if n > 0 => {
                    sink.write_bytes(&read_buf[..n]);
                }
                Ok(0) if ch.eof() => {
                    sink.log("\r\n[Session] Remote host closed connection.\r\n");
                    break;
                }
                _ => {}
            }
        } else {
            break;
        }
        drop(ctx);
        std::thread::sleep(Duration::from_millis(15));
    }
}


