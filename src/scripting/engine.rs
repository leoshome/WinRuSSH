use std::sync::Arc;
use crossbeam_channel::{Receiver, Sender};
use mlua::prelude::*;
use parking_lot::Mutex;

use crate::session::{SendHwnd, UiSink};
use crate::terminal::buffer::TerminalBuffer;
use super::ssh_module::{register_ssh_module, run_post_script_interactive, SharedSshContext};

pub fn create_lua_runtime(sink: UiSink, input_rx: Option<Receiver<Vec<u8>>>) -> Lua {
    create_lua_runtime_with_context(sink, input_rx).0
}

fn create_lua_runtime_with_context(
    sink: UiSink,
    input_rx: Option<Receiver<Vec<u8>>>,
) -> (Lua, Arc<Mutex<SharedSshContext>>) {
    let lua = Lua::new();

    // Custom print implementation routing to TerminalBuffer
    let print_sink = sink.clone();
    let print_fn = lua
        .create_function(move |_, args: LuaMultiValue| {
            let mut parts = Vec::new();
            for val in args {
                parts.push(match val {
                    LuaValue::String(s) => s.to_str()?.to_string(),
                    LuaValue::Integer(i) => i.to_string(),
                    LuaValue::Number(n) => n.to_string(),
                    LuaValue::Boolean(b) => b.to_string(),
                    LuaValue::Nil => "nil".to_string(),
                    _ => format!("{:?}", val),
                });
            }
            let output = parts.join("\t");

            if print_sink.has_buffer() {
                print_sink.log(&output);
            } else {
                println!("{}", output);
            }
            Ok(())
        })
        .expect("Failed to register print function");

    lua.globals().set("print", print_fn).unwrap();

    let context = Arc::new(Mutex::new(SharedSshContext::new(sink, input_rx)));
    register_ssh_module(&lua, context.clone()).expect("Failed to register ssh module");

    (lua, context)
}

pub fn spawn_lua_worker(
    script_path: String,
    script_args: Vec<String>,
    buffer: Arc<Mutex<TerminalBuffer>>,
    hwnd_notify: Option<SendHwnd>,
    tab_id: usize,
) -> (Sender<Vec<u8>>, std::thread::JoinHandle<()>) {
    let (tx, rx) = crossbeam_channel::unbounded();
    let sink = UiSink::new(Some(buffer), hwnd_notify, tab_id);

    let handle = std::thread::spawn(move || {
        let panic_sink = sink.clone();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            run_lua_script(script_path, script_args, rx, sink, tab_id);
        }));
        if let Err(p) = result {
            let msg = p
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| p.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown panic".into());
            panic_sink.log(&format!("\x1b[31m[Lua Panic] {}\x1b[0m", msg));
            panic_sink.notify_status(false);
        }
    });

    (tx, handle)
}

fn run_lua_script(
    script_path: String,
    script_args: Vec<String>,
    rx: Receiver<Vec<u8>>,
    sink: UiSink,
    _tab_id: usize,
) {
    {
        let (lua, context) = create_lua_runtime_with_context(sink.clone(), Some(rx));

        // Standard Lua convention: arg[0] = script, arg[1..] = CLI arguments
        if let Ok(t) = lua.create_table() {
            let _ = t.set(0, script_path.as_str());
            for (i, a) in script_args.iter().enumerate() {
                let _ = t.set(i + 1, a.as_str());
            }
            let _ = lua.globals().set("arg", t);
        }

        sink.log(&format!("[INFO] Running Lua Script: {}", script_path));
        sink.notify_status(true);

        match std::fs::read_to_string(&script_path) {
            Err(e) => sink.error(&format!("Failed to read script {}: {}", script_path, e)),
            Ok(content) => match lua.load(&content).exec() {
                Err(e) => sink.log(&format!("\x1b[31m[Lua Error] {}\x1b[0m", e)),
                Ok(()) => sink.log("\x1b[32m[INFO] Lua Script finished successfully.\x1b[0m"),
            },
        }

        // Keep the SSH session interactive after the script completes if still connected
        run_post_script_interactive(&context);

        sink.notify_status(false);
    }
}
