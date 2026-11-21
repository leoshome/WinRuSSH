-- scripts/select_file_login_send_cmd.lua
-- Pick a command .txt via PowerShell, log in to test.rebex.net, send each line.
local ps = [[powershell -NoProfile -STA -Command "Add-Type -AssemblyName System.Windows.Forms; $d = New-Object System.Windows.Forms.OpenFileDialog; $d.Filter = 'Text (*.txt)|*.txt'; if ($d.ShowDialog() -eq 'OK') { $d.FileName }"]]
local pipe = io.popen(ps, "r")
local path = pipe and pipe:read("*l")
if pipe then pipe:close() end
path = path and path:gsub("\r$", "")
if not path or path == "" then print("[Script] No file selected.") return end

local f, e = io.open(path, "r")
if not f then print("[Error] " .. tostring(e)) return end

local ok, err = ssh.connect({ host = "test.rebex.net", port = 22, username = "demo", password = "password" })
if not ok then print("[Error] Login failed: " .. tostring(err)) f:close() return end

for line in f:lines() do
    line = line:gsub("\r$", "")
    if line ~= "" then
        if not ssh.expect("$", 8000) then print("[Error] Prompt timeout") break end
        ssh.send(line .. "\n")
    end
end
f:close()
ssh.expect("$", 8000)
print("[Script] Done.")
