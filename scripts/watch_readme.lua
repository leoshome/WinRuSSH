-- scripts/watch_readme.lua
-- Usage: winrussh.exe scripts\watch_readme.lua [host] [port] [username] [password]
-- Watches terminal output: sends "OK" whenever "readme" appears,
-- or sends Enter every 20 seconds if "readme" is not seen.

print("==================================================")
print("  Watch 'readme' -> 'OK' / 20s -> Enter")
print("==================================================")

local target = {
    host       = (arg and arg[1]) or "test.rebex.net",
    port       = tonumber(arg and arg[2]) or 22,
    username   = (arg and arg[3]) or "demo",
    password   = (arg and arg[4]) or "password",
    accept_new = true,
}

print(string.format("[Script] Connecting to %s:%d (user: %s)...", target.host, target.port, target.username))
local ok, err = ssh.connect(target)
if not ok then
    print("[Error] Connection failed: " .. tostring(err))
    return
end

print("[Script] Connected. Watching for 'readme' (20s timeout -> Enter)...")

while true do
    local idx = ssh.expect("readme", 20000)
    if idx then
        print("[Script] Detected 'readme' -> sending 'OK'")
        if not ssh.send("OK\n") then
            print("[Script] Channel closed, stopping.")
            break
        end
    else
        print("[Script] 20s elapsed -> sending Enter")
        if not ssh.send("\n") then
            print("[Script] Channel closed, stopping.")
            break
        end
    end
end
