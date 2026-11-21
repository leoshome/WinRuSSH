-- scripts/rebex_test.lua
print("==================================================")
print("  Test Script: Connecting to test.rebex.net")
print("==================================================")

local target = {
    host = "test.rebex.net",
    port = 22,
    username = "demo",
    password = "password",
}

print(string.format("[Script] Connecting to %s:%d (user: %s)...", target.host, target.port, target.username))

-- 1. Establish SSH connection and authenticate
local ok, err = ssh.connect(target)

if not ok then
    print(string.format("[Script] Connection failed: %s", tostring(err)))
    return
end

print("[Script] Connected! Waiting for login prompt '$'...")

-- 2. Wait for login prompt $
if ssh.expect("$", 8000) then
    print("[Script] Received '$' prompt, sending command #1: dir")
    ssh.send("dir\n")

    -- 3. Wait for '$' prompt after dir finishes
    if ssh.expect("$", 8000) then
        print("[Script] dir completed, received '$' prompt, sending command #2: ls")
        ssh.send("ls\n")

        -- 4. Wait for '$' prompt after ls finishes
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
