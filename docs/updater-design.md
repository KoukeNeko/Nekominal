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

1. 使用者按「更新」。舊 Nekominal 把 `updater.exe` 複製到 `%LOCALAPPDATA%\Nekominal\updater\<sha256 前 12 碼>\` 再啟動它（同樣避開 app 資料夾），兩者以只有使用者本人能開的 named pipe 連線（與自動化 API 相同的 SDDL）。（實作時沒有先問 Scoop 有沒有新版本：沒有的話就走下面「沒有新版本」的路，分頁一樣接回。）
2. **交接**：舊 Nekominal 對每個窗格，用 `DuplicateHandle` 把下列 handle 複製進 updater：
   - job、shell 程序
   - ConPTY 的輸入寫端、輸出讀端
   - pseudoconsole 內部的三個 handle（signal 管線、reference、console host 程序）
   
   再送出工作階段（版面沿用現有的工作階段格式；每個窗格的 session id、標題、大小，和它顯示的內容：捲動歷史、畫面、游標和程式設定的模式，見下方「實作」）。updater 確認全部收到後，舊 Nekominal 結束；它關掉的只是自己那份 handle，job 和 shell 仍由 updater 持有，所以都還活著。
3. **更新**：updater 開一個小視窗，顯示「Nekominal 0.2.3 → 0.2.4」和 Scoop 的輸出，執行 `scoop update; scoop update nekominal`。Claude Code 的 hooks 會短暫從 app 資料夾執行 `Nekominal.exe`，碰到「仍在執行」就等一下重試，最多 30 秒。
4. **接回**：updater 啟動新版 Nekominal，帶上 `--adopt <pipe>`。新版連回 updater，取得複製進它自己程序的 handle 和工作階段，照原本的版面重建視窗和分頁：
   - 每個窗格接上原本的 shell，而不是開新的。
   - 先放回交接時的畫面（逐列的畫面和捲動歷史、游標、模式），再接著讀管線裡累積的新輸出：程式接下來的輸出會落在它原本以為的位置。
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

## 實作

- **pseudoconsole 的重建**：釘住的 conpty.dll（1.25.260930003）的 `PseudoConsole` 結構是 `{ hSignal, hPtyReference, hConPtyProcess }`，以 `HeapAlloc(GetProcessHeap())` 配置、`ClosePseudoConsole` 以 `HeapFree` 釋放。接手的一方不自己配置，而是呼叫同一個 DLL 匯出的 `ConptyPackPseudoConsole(hProcess, hRef, hSignal, &hpc)`，配置方式自然一致，Resize 和 Close 照常可用。交出的一方需要讀出三個 handle，這是唯一依賴內部結構的地方；`nekominal-pty` 的測試 `the_pinned_conpty_keeps_a_pseudoconsole_as_signal_reference_and_console_process` 用 Pack 放入三個已知 handle 再讀回、檢查順序，並確認 `ClosePseudoConsole` 會關掉它們；換了 conpty.dll 而結構不同時這個測試會失敗。交出前也會檢查 signal 是管線、console 是程序。系統內建的 kernel32 ConPTY 沒有 `ConptyPackPseudoConsole`，所以只有隨附的 conpty.dll 能接手（Scoop 安裝一定是隨附的）。
- **交出的一方不關任何東西**：每個窗格先暫停讀取（讀取執行緒的 `ReadFile` 用 `CancelSynchronousIo` 取消，不會吃掉資料），等已讀到的輸出都進了模擬器，才取畫面和 handle。交接成功後以 `TerminateProcess` 結束自己：不跑任何解構、不呼叫 `ClosePseudoConsole`（那會讓 console host 結束），系統只關掉它自己那份 handle。交接失敗時每個窗格恢復讀取，畫面上說明原因。
- **畫面**：`TerminalCore::handoff_screen` 把主緩衝區的捲動歷史和畫面逐列寫成 VT（畫面的空白列也保留、最後一列不換行），接著游標位置；程式在替代畫面時再加上 `?1049h`、替代畫面的內容和游標；最後是和新終端機不同的模式（游標鍵、滑鼠、聚焦回報、bracketed paste、隱藏游標、kitty 鍵盤旗標、ConPTY 的 win32-input-mode）。接手時原樣餵入同樣大小的新模擬器，所以畫面一格不差。沒有保留：捲動區域、字元集、目前的畫筆。
- **訊息**：`nekominal-pty` 的 `handoff` 模組，一行一則 JSON（`give`、`take`、`windows`、`adopted`、`ok`、`error`），每則帶協定版本 `v`。版本不符時接手方不使用 handle（由 updater 結束那些工作階段），只用文字還原並開新 shell。
- **其他執行中的 Nekominal**：updater 從 `%LOCALAPPDATA%\Nekominal\api\<pid>` 找到它們，經自動化 API 送 `{"cmd":"hand-off","pipe":…}`（只接受 updater 的 pipe 名稱，並確認對方程式是 `updater.exe`）。沒有 API 的（設定關掉、或以系統管理員身分執行）會收到登錄的視窗訊息 `NekominalUpdateClose`（提升權限的視窗用 `ChangeWindowMessageFilterEx` 允許這一則），照以前的方式儲存並關閉；updater 最多等 90 秒。從系統管理員視窗按「更新」時，沿用以前的 PowerShell 腳本。
- **Scoop**：updater 執行 app 所在 Scoop 根目錄自己的 `apps\scoop\current\bin\scoop.ps1`（並設 `SCOOP` 為該根目錄），沒有才用 PATH 上的 `scoop`；所以在另一個 Scoop 根目錄裡測試不會動到使用者的 Scoop。
- **接手**：新版從 `--adopt <pipe>` 取得視窗，每個視窗用原本的版面重建；窗格保留 session id（`agent::reserve_session` 讓新的 id 不會和它撞上），標題也帶過來。只有 updater 自己啟動的程序能取得 handle。接手後每個窗格縮一列再放回，讓全螢幕程式重畫。舊 shell 環境裡的 `NEKOMINAL_PIPE` 指向舊程序的 pipe，`nekominal cli` 找不到它時改連最新的執行中實例。
- **updater 結束**：新版回報接手了哪些工作階段之後，updater 關掉自己的 handle，沒被接手的工作階段（job）被結束；新版沒在 60 秒內接手（或啟動後就結束）時，先結束它，再試下一個（Scoop 現在的版本 → 交出時的版本資料夾 → 交出的那個 exe）；都失敗就結束所有工作階段，視窗顯示原因。

## 測試

- 單元測試：交接訊息的序列化和版本（`nekominal-pty` `handoff::tests`）、handle 複製失敗時的清理、updater 的參數和 Scoop 輸出判斷（`nekominal-updater`）、接手方關掉沒用到的 handle（`nekominal-shell` `handoff::tests`）、畫面交接後一格不差（`nekominal-term` `tests/restore.rs` 的 `a_handed_over_*`）。
- pseudoconsole 結構：`the_pinned_conpty_keeps_a_pseudoconsole_as_signal_reference_and_console_process`。
- 整合測試：`nekominal-pty` 的 `tests/handoff.rs` 在子程序開一個 ConPTY 窗格執行 `cmd /k ping -t`，把它交給父程序後子程序以 `TerminateProcess` 結束；父程序確認 cmd 和 ping 還在、接手後 ping 的輸出繼續、Ctrl+C、resize（`mode con` 看到新大小）和 `exit 5` 都正常。`nekominal-updater` 的 `the_holder_keeps_the_sessions_given_hands_them_to_its_taker_alone_and_ends_those_not_adopted` 走過 updater 的 pipe：give、拒絕非 updater 啟動的程序、take、adopted，沒被接手的工作階段被結束。`nekominal-pane` 的 `tests/handoff_pane.rs` 確認接手的窗格是同一個 shell（之前設的變數還在）。
- 真實測試：在沙箱裡用一個本機 Scoop bucket，從 0.2.3 更新到測試版，分頁裡同時跑著 Claude Code。

## 另外：Nekominal.exe 的圖示

`apps\nekominal\<版本>\Nekominal.exe` 本身有嵌入圖示，16 到 256 px 都正常。沒有圖示的是 Scoop 產生的 `scoop\shims\nekominal.exe`：它是 Scoop 通用的啟動器，Scoop 不提供給它設定圖示的方式。在命令列輸入 `nekominal`，或從 PATH 啟動時，工作管理員裡就會多出這個沒有圖示的程序。可行的做法是在 Scoop manifest 改用 `env_add_path`，把 app 資料夾加進 PATH，取代 `bin` 產生的 shim，讓 `nekominal` 直接執行真正的 Nekominal.exe。
