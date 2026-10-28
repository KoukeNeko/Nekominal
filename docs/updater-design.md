# updater.exe：不中斷工作的更新

## 目標

在 Nekominal 裡按「更新」之後，Nekominal 換成新版本，而每個分頁裡正在跑的 shell、agent 和指令都還活著，接回新版本的同一個位置：不用重開分頁、不用重新啟動 Claude Code，捲動歷史也還在。

## 現在為什麼做不到

1. **分頁的程序跟著 Nekominal 一起死。** 每個窗格的 shell 放在一個 `KILL_ON_JOB_CLOSE` 的 job 裡，ConPTY 的兩條管線和 pseudoconsole 都由 Nekominal 持有。Nekominal 一結束，job 關閉，shell 和 agent 全部被終止。
2. **Scoop 拒絕更新。** `scoop update` 會檢查有沒有任何程序的執行檔在 `apps\nekominal\` 底下（`lib\install.ps1` 的 `test_running_process`），有就跳過更新。每個分頁的 `OpenConsole.exe` 正是從 `apps\nekominal\<版本>\` 執行的。
3. **現在的更新腳本**只會等 Nekominal 關閉、更新、再啟動，所以分頁裡的程式一定會中斷。

## 設計

### 1. ConPTY 從 app 資料夾外執行

啟動時把隨附的 `conpty.dll` 和 `OpenConsole.exe` 複製到 `%LOCALAPPDATA%\Nekominal\conpty\<sha256 前 12 碼>\`（內容沒變就不重複複製），從那裡載入。這樣分頁的 console host 不在 `apps\nekominal\` 底下，Scoop 不會因為它們而拒絕更新。舊版本的資料夾用不到時，由下一次啟動清掉。

### 2. updater.exe：交接期間保管分頁

新的小程式 `updater.exe`（workspace 新增 `crates/nekominal-updater`，只用 Win32，不含繪圖），隨 zip 一起發布。它的工作是在新舊版本之間，暫時持有所有分頁：

1. 使用者按「更新」。舊 Nekominal 先確認 Scoop 有新版本，把 `updater.exe` 複製到 `%LOCALAPPDATA%\Nekominal\updater\` 再啟動它（同樣避開 app 資料夾），兩者以只有使用者本人能開的 named pipe 連線（與自動化 API 相同的 SDDL）。
2. **交接**：舊 Nekominal 對每個窗格，用 `DuplicateHandle` 把下列 handle 複製進 updater：
   - job、shell 程序
   - ConPTY 的輸入寫端、輸出讀端
   - pseudoconsole 內部的三個 handle（signal 管線、reference、console host 程序）
   
   再寫出工作階段（版面、每個窗格的文字和 session id，沿用現有的工作階段還原）。updater 確認全部收到後，舊 Nekominal 結束；它關掉的只是自己那份 handle，job 和 shell 仍由 updater 持有，所以都還活著。
3. **更新**：updater 開一個小視窗，顯示「Nekominal 0.2.3 → 0.2.4」和 Scoop 的輸出，執行 `scoop update; scoop update nekominal`。Claude Code 的 hooks 會短暫從 app 資料夾執行 `Nekominal.exe`，碰到「仍在執行」就等一下重試，最多 30 秒。
4. **接回**：updater 啟動新版 Nekominal，帶上 `--adopt <pipe>`。新版連回 updater，取得複製進它自己程序的 handle 和工作階段，照原本的版面重建視窗和分頁：
   - 每個窗格接上原本的 shell，而不是開新的。
   - 先放回保存的文字，再接著讀管線裡累積的新輸出。
   - 對每個窗格送一次 resize，讓全螢幕程式（Claude Code、vim）重畫整個畫面。
   - 窗格沿用原本的 session id，所以 shell 裡的 `NEKOMINAL_SESSION`、agent 的 hook 和 status line 檔都能接著用。
5. updater 確認新版已接手後結束。

### 3. 交接期間的輸出

交接和更新期間沒有人讀管線，管線滿了之後，分頁裡的程式寫輸出時會暫停，等新版開始讀就繼續。輸出不會遺失，只是暫停幾秒到幾十秒，長短看 Scoop 下載有多快。

### 4. 失敗時

- **Scoop 更新失敗或沒有新版本**：updater 改啟動原本的版本，同樣用 `--adopt` 接回，畫面上顯示 Scoop 的錯誤。分頁一樣不中斷。
- **新版本接不回某個窗格**（例如交接的格式改了）：那個窗格用保存的文字還原，開一個新的 shell，並說明原本的程式已結束。updater 結束時會關掉它還持有的 job，不留沒人管的程序。
- **新版本啟動失敗**：updater 改啟動原本版本的 exe（Scoop 會保留舊版本資料夾），接回分頁。
- **舊 Nekominal 在交接中途當掉**：已複製給 updater 的 handle 仍然有效，updater 一樣繼續更新並接回。

### 5. 不在範圍內

- **以系統管理員身分執行的視窗**：一般權限的程序無法接手提升權限的 handle，這些視窗在更新時照現在的方式關閉，再用工作階段還原文字。畫面上會先說明。
- **跨重新開機保留分頁**：這是另一個功能（常駐的 mux 伺服器）。這份設計只處理一次更新的交接。

## 風險

- **pseudoconsole 的內部結構**：handle 交接依賴 `conpty.dll` 內部的 `PseudoConsole` 結構（三個 handle），這不是公開 API。Nekominal 綁定並驗證自己隨附的 conpty.dll 版本，所以可以固定這個結構，並加測試在升級 conpty.dll 時抓出差異。
- **交接格式的版本**：交接訊息帶版本號。新版看不懂舊格式時，退回「還原文字、開新 shell」。

## 測試

- 單元測試：交接訊息的序列化、handle 清單、失敗時的退回路徑。
- 整合測試：在測試中開一個 ConPTY 窗格執行 `ping -t`，把它交接給另一個程序，確認 ping 沒有中斷，輸出接得上。
- 真實測試：在沙箱裡用一個本機 Scoop bucket，從 0.2.3 更新到測試版，分頁裡同時跑著 Claude Code。

## 另外：Nekominal.exe 的圖示

`apps\nekominal\<版本>\Nekominal.exe` 本身有嵌入圖示，16 到 256 px 都正常。沒有圖示的是 Scoop 產生的 `scoop\shims\nekominal.exe`：它是 Scoop 通用的啟動器，Scoop 不提供給它設定圖示的方式。在命令列輸入 `nekominal`，或從 PATH 啟動時，工作管理員裡就會多出這個沒有圖示的程序。可行的做法是在 Scoop manifest 改用 `env_add_path`，把 app 資料夾加進 PATH，取代 `bin` 產生的 shim，讓 `nekominal` 直接執行真正的 Nekominal.exe。
