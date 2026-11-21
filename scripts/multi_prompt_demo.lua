-- scripts/multi_prompt_demo.lua
-- Demo: Multi-prompt table waiting + capturing intermediate output text

print("==================================================")
print("  Demo: Multi-Prompt Expect & Output Capture")
print("==================================================")

local target = {
    host = "test.rebex.net",
    port = 22,
    username = "demo",
    password = "password",
}

print(string.format("[Script] Connecting to %s...", target.host))
local ok, err = ssh.connect(target)
if not ok then
    print("[Script] Connection failed: " .. tostring(err))
    return
end

-- 1. Wait for multiple candidate prompts simultaneously (e.g., $, #, or >)
--    Return values:
--    - match_idx: 1-based index of matched prompt (nil on timeout)
--    - output: All text received between the previous prompt and this match
--    - matched: The exact prompt string that matched
local prompts = { "$", "#", ">", "Password:" }
local match_idx, output, matched = ssh.expect(prompts, 8000)

if match_idx then
    print(string.format("[Script] Login succeeded! Matched prompt #%d: '%s'", match_idx, matched))
    print("[Script] === Welcome Banner Received on Login ===")
    print(output)
    print("====================================\n")

    -- 2. Send command 'dir' and capture its exact output
    print("[Script] Sending command: dir")
    ssh.send("dir\n")

    local dir_idx, dir_output = ssh.expect(prompts, 5000)
    if dir_idx then
        print("[Script] * Captured 'dir' command output *:")
        print("--------------------------------------------")
        print(dir_output)
        print("--------------------------------------------\n")
    end

    -- 3. Send command 'ls' and capture its exact output
    print("[Script] Sending command: ls")
    ssh.send("ls\n")

    local ls_idx, ls_output = ssh.expect(prompts, 5000)
    if ls_idx then
        print("[Script] * Captured 'ls' command output *:")
        print("--------------------------------------------")
        print(ls_output)
        print("--------------------------------------------\n")
    end
else
    print("[Script] Timed out waiting for prompt!")
end

print("==================================================")
print("  Test Complete!")
print("==================================================")
