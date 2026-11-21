-- scripts/rebex_test_arg.lua
-- Usage: winrussh.exe scripts\rebex_test_arg.lua [host] [port] [username] [password]
-- Missing args fall back to the rebex demo defaults.
print("==================================================")
print("  Test Script (arg version): Connecting via arguments")
print("==================================================")

local target = {
    host     = "test.rebex.net",
    port     = 22,
    username = arg and arg[1],
    password = arg and arg[2],
}

print(string.format("[Script] Connecting to %s:%d (user: %s)...", target.host, target.port, target.username))

local ok, err = ssh.connect(target)
if not ok then
    print(string.format("[Script] Connection failed: %s", tostring(err)))
    return
end

print("[Script] Connected! Waiting for login prompt '$'...")

if ssh.expect("$", 8000) then
    print("[Script] Received '$' prompt, sending command #1: dir")
    ssh.send("dir\n")

    if ssh.expect("$", 8000) then
        print("[Script] dir completed, received '$' prompt, sending command #2: ls")
        ssh.send("ls\n")

        if ssh.expect("$", 8000) then
            print("[Script] ls completed! All commands executed successfully.")
        else
            print("[Script] Timed out waiting for '$' after ls.")
        end
    else
        print("[Script] Timed out waiting for '$' after dir.")
    end
else
    print("[Script] Timed out waiting for login prompt '$'.")
end

print("==================================================")
print("  Test task finished.")
print("==================================================")
