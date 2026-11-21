use winrussh::scripting::create_lua_runtime;
use winrussh::session::UiSink;

#[test]
fn test_lua_runtime_and_ssh_module() {
    let lua = create_lua_runtime(UiSink::new(None, None, 0), None);
    let script = r##"
        print("Testing Lua Engine")
        assert(type(ssh.sleep) == "function")
        assert(type(ssh.send) == "function")
        assert(type(ssh.expect) == "function")
        assert(ssh.interactive == nil)
        ssh.sleep(10)
    "##;
    assert!(lua.load(script).exec().is_ok());
}

#[test]
fn test_lua_expect_multi_prompt_and_capture() {
    let lua = create_lua_runtime(UiSink::new(None, None, 0), None);
    let script = r##"
        -- Test multi-prompt table syntax (returns nil, captured, nil on timeout)
        local idx, output, matched = ssh.expect({ "$", "#", ">", "Password:" }, 50)
        assert(idx == nil)
        assert(type(output) == "string")
        assert(matched == nil)

        -- Test single-string prompt syntax
        local idx2, output2 = ssh.expect("$", 50)
        assert(idx2 == nil)
        assert(type(output2) == "string")
    "##;
    assert!(lua.load(script).exec().is_ok());
}

