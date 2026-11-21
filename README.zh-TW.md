# WinRuSSH — 原生 Windows SSH 終端機與 Lua 自動化控制台

[English](README.md) | **繁體中文**

這是一款極致輕量的原生 Windows SSH 終端機與自動化控制台，使用 **Rust** 語言搭配微軟官方 **`windows-rs`**（`windows` 0.62）、**`ssh2`** (libssh2) 以及內嵌 **Lua 5.4 (`mlua`)** 打造。

---

## 核心特色

- **原生 Win32 GUI**：以 `windows-rs` 打造，支援 High-DPI v2 與 ClearType，並在視窗最小化或縮放時保留終端機緩衝區文字不截斷，同時於縮小高度時裁切多餘尾部空白行。
- **多分頁工作區**：支援同時多個 SSH 分頁與 Lua 腳本任務。
- **Quick Connect 快速連線 (`F2`)**：記憶上次連線設定，支援互動式登入提示與 `known_hosts` 指紋驗證。
- **Lua 自動化引擎**：免編譯執行 `.lua` 腳本，支援 `ssh.connect`、`ssh.expect`、`ssh.send`、`ssh.sleep` 與 `ssh.read`。
- **人機並行控制**：腳本執行期間與結束後皆可直接以鍵盤輸入，與腳本同時操作同一連線。

---

## 快捷鍵與操作說明

| 快捷鍵 / 操作 | 功能 |
| :--- | :--- |
| **`滑鼠左鍵拖曳`** | **拖曳反白選取終端文字 (Classic Blue High-Contrast Selection)** |
| **`滑鼠右鍵`** | **開啟快顯選單：複製 (Copy) / 貼上 (Paste) / 開始或停止分頁日誌 (Tab Log)** |
| **`Ctrl + C`** | 若有選取文字則**複製到剪貼簿**；若無選取則發送終端中斷訊號（`SIGINT`） |
| **`Ctrl + V`** / **`Shift + Insert`** | **將系統剪貼簿文字貼上至終端** |
| **`Ctrl + +` / `Ctrl + =`** | **放大字型大小 (Zoom In)** |
| **`Ctrl + -`** | **縮小字型大小 (Zoom Out)** |
| **`Ctrl + 0`** | **重設字型大小為預設 (16px)** |
| **`F2`** | 開啟 Quick Connect 快速連線對話框（支援 `Tab` 跳躍與自動記憶） |
| **`F5`** | 選擇並執行 Lua 自動化腳本 (`.lua`) |
| **`Ctrl + T`** | 建立新分頁 (New Tab) |
| **`Ctrl + W`** | 關閉當前分頁 (Close Tab) |
| **`Ctrl + Tab`** | 切換至下一分頁 |
| **`Ctrl + Shift + Tab`** | 切換至上一分頁 |
| **`滑鼠滾輪`** | 滾動檢視終端歷史輸出緩衝區 |

---

## 撰寫自動化腳本 (`.lua`)

腳本為標準純文字 Lua 檔案，無須編譯即可即時載入執行：

```lua
-- example.lua
print("==================================================")
print("  WinRuSSH Lua Automation Demo")
print("==================================================")

local target = {
    host = "192.168.1.100",
    port = 22,
    username = "admin",
    password = "secret_password",
}

print(string.format("Connecting to %s:%d as %s...", target.host, target.port, target.username))
local ok, err = ssh.connect(target)

if not ok then
    print("[Error] Failed: " .. tostring(err))
    return
end

print("Connected! Waiting for prompt...")
if ssh.expect("$", 5000) or ssh.expect("#", 5000) then
    print("Sending inspect command: uname -a")
    ssh.send("uname -a\n")
    ssh.expect("$", 3000)

    print("Pausing 2s - you can type with the keyboard anytime...")
    ssh.sleep(2000)

    print("Back in Lua script! Performing cleanup...")
    ssh.send("exit\n")
end

print("Automation script complete.")
```

### 從命令列啟動

```bat
winrussh.exe [--script] <script.lua> [參數...]

winrussh.exe scripts\rebex_test.lua
winrussh.exe scripts\rebex_test_arg.lua demo password
winrussh.exe scripts\select_file_login_send_cmd.lua
```

程式會開啟新的 `Lua: <名稱>` 分頁並執行腳本。Lua 內 `arg[0]` 為腳本路徑,`arg[1..]` 為額外參數。

### 內附腳本 (`scripts/`)

| 腳本 | 用法 |
| :--- | :--- |
| `rebex_test.lua` | 登入 `test.rebex.net` (demo/password) 並執行 `dir`、`ls`,無需參數。 |
| `rebex_test_arg.lua` | 同上,但帳密由參數提供:`arg[1]` = 使用者名稱,`arg[2]` = 密碼。 |
| `select_file_login_send_cmd.lua` | 以 PowerShell 開啟檔案對話框選擇指令 `.txt`,登入 `test.rebex.net`,每遇到 `$` 提示字元就送出檔案中的一行指令。 |
| `multi_prompt_demo.lua` | 以 `ssh.expect({...})` 同時等待多個提示字元的範例。 |
| `watch_readme.lua` | 持續監看輸出：每次偵測到 `readme` 就送出 `OK`，否則每 20 秒送出 Enter。可選參數：`[host] [port] [username] [password]`。 |

### Lua `ssh` 模組 API 參考
- `ssh.connect({ host, port, username, password, key_path, passphrase, accept_new })`：建立 SSH 連線。
  - `accept_new`（選填布林值，預設 `false`）：設為 `true` 時，遇到未知主機會自動信任並寫入 `known_hosts`（若公鑰與紀錄不符仍一律拒絕）；設為 `false` 時，需先透過互動分頁信任該主機。
- `ssh.send(string)`：發送指令字串（需包含 `\n`）。
- `ssh.expect(pattern_or_table, timeout_ms)`：
  - 支援單一字串（如 `"$"`）或多種候選 Prompt 陣列（如 `{"$", "#", ">", "Password:"}`）。
  - 回傳值：`(match_index, captured_text, matched_pattern)`
    - `match_index`：命中的 Prompt 索引（`1, 2, 3...`；逾時回傳 `nil`）。
    - `captured_text`：**上次 Prompt 到本次命中之間所收到的所有輸出文字**。
    - `matched_pattern`：實際命中的 Prompt 字串。
- `ssh.read(timeout_ms)`：讀取目前累積的終端輸出內容。
- `ssh.sleep(ms)`：暫停指定毫秒數（期間仍會轉送手動鍵盤輸入與遠端輸出）。

---

## 從原始碼建置

### 系統需求
- Windows 10 或 Windows 11 (x86_64)
- Rust 工具鏈 (`rustup install stable`)
- Visual Studio C++ Build Tools (MSVC)

### 建置指令
```bash
# Debug 建置
cargo build

# 最佳化 Release 建置（執行檔約 ~0.75 MB）
cargo build --release
```
編譯完成的執行檔位於 `target\release\winrussh.exe`。
