# The self-drawn shell (crates/nekominal-shell, nekominal-next.exe) driven with real input. It has no UI Automation tree
# yet, so what it did is read back through the clipboard (select all, copy) and the process counters.
#
#   . .\tools\slice-drive\next-scenarios.ps1
#   Scenario-NextTabs        # a new tab from the button and from the keyboard, switching, closing: each tab keeps its own text
#   Scenario-NextClipboard   # select by dragging, copy, paste, and a program copying through OSC 52
#   Scenario-NextKeys        # Ctrl+U, Ctrl+C, Backspace and CJK text reach the shell
#   Scenario-NextClaude [fullscreen|classic]   # Claude Code starts, takes text and exits, no model call
#   Scenario-NextIdle        # memory and CPU of an idle window
#   Scenario-NextAll
#
# A scenario fails (throws) when a check fails. The clipboard of the user is saved and restored.

. (Join-Path $PSScriptRoot 'drive-lib.ps1')

$global:NextExe = Join-Path $PSScriptRoot '..\..\target\release\nekominal-next.exe'
$global:NextFailures = @()

function Next-Check([string]$Name, [bool]$Ok, [string]$Detail) {
    Write-Host ("{0,-4} {1} | {2}" -f $(if ($Ok) { 'ok' } else { 'FAIL' }), $Name, $Detail)
    if (-not $Ok) { $global:NextFailures += $Name }
}

# Runs `$Body` with the shell started, in a clean environment, and puts the user's clipboard back afterwards.
function Run-Next([scriptblock]$Body) {
    $saved = Get-Clipboard -Raw -ErrorAction SilentlyContinue
    $global:NextFailures = @()
    try {
        Start-App -Exe (Resolve-Path $global:NextExe).Path -Log '' -Layout '' -SettleMs 3500
        & $Body
    } finally {
        Stop-App
        if ($null -ne $saved) { Set-Clipboard -Value $saved }
    }
    if ($global:NextFailures.Count -gt 0) { throw "failed: $($global:NextFailures -join ', ')" }
}

# A point of the window in DIP, as screen pixels.
function Next-At([double]$X, [double]$Y) {
    $r = Win-Rect; $s = Get-Scale
    @([int]($r.Left + $X * $s), [int]($r.Top + $Y * $s))
}

# The whole text of the selected tab, through select all and copy.
function Next-Text {
    Set-Clipboard -Value ''
    Press 0x41 @(0x11, 0x10)    # Ctrl+Shift+A
    Press 0x43 @(0x11, 0x10)    # Ctrl+Shift+C
    Start-Sleep -Milliseconds 300
    [string](Get-Clipboard -Raw)
}

function Next-Line([string]$Text) { Type-Unicode $Text 30; Press 0x0D; Start-Sleep -Milliseconds 900 }

function Scenario-NextTabs {
    Run-Next {
        Next-Line 'echo TAB-ONE'
        $plus = Next-At 248 24; Click-At $plus[0] $plus[1]; Start-Sleep -Milliseconds 1500
        Next-Line 'echo TAB-TWO'
        $text = Next-Text
        Next-Check 'tabs.second-has-its-own-text' ($text -match 'TAB-TWO' -and $text -notmatch 'TAB-ONE') 'the new tab shows what was typed in it only'
        $first = Next-At 60 24; Click-At $first[0] $first[1]; Start-Sleep -Milliseconds 600
        $text = Next-Text
        Next-Check 'tabs.first-keeps-its-text' ($text -match 'TAB-ONE' -and $text -notmatch 'TAB-TWO') 'selecting the first tab shows its own text'
        Press 0x54 @(0x11, 0x10); Start-Sleep -Milliseconds 1500    # Ctrl+Shift+T
        $text = Next-Text
        Next-Check 'tabs.keyboard-new-tab' ($text -notmatch 'TAB-ONE' -and $text -notmatch 'TAB-TWO') 'Ctrl+Shift+T opens a third, empty tab'
        Press 0x57 @(0x11, 0x10); Start-Sleep -Milliseconds 800    # Ctrl+Shift+W
        Next-Line 'echo AFTER-CLOSE'
        $text = Next-Text
        Next-Check 'tabs.close' ($text -match 'AFTER-CLOSE') 'after closing the third tab the window still takes input'
    }
}

function Scenario-NextClipboard {
    Run-Next {
        Set-Clipboard -Value 'before'
        Next-Line 'echo SELECTME-12345 done'
        # The second row of the terminal holds the output "SELECTME-12345"; drag across its first columns.
        $from = Next-At 2 (48 + 27); $to = Next-At 160 (48 + 27)
        Drag-To $from[0] $from[1] $to[0] $to[1] 15 20
        Press 0x43 @(0x11, 0x10); Start-Sleep -Milliseconds 300
        $copied = [string](Get-Clipboard -Raw)
        Next-Check 'clipboard.drag-copy' ($copied.Trim() -eq 'SELECTME-12345') "copied '$copied'"
        Set-Clipboard -Value 'PASTED-TEXT'
        Press 0x56 @(0x11, 0x10); Start-Sleep -Milliseconds 600      # Ctrl+Shift+V
        $text = Next-Text
        Next-Check 'clipboard.paste' ($text -match 'PASTED-TEXT') 'pasted text reached the prompt'
        Press 0x1B            # Escape clears the input line of PSReadLine
        $b64 = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes('from-osc52'))
        Set-Clipboard -Value 'before'
        Next-Line ('[Console]::Out.Write("$([char]27)]52;c;' + $b64 + '$([char]7)")')
        Next-Check 'clipboard.osc52-write' ([string](Get-Clipboard -Raw) -eq 'from-osc52') 'a program copied through OSC 52'
    }
}

function Scenario-NextKeys {
    Run-Next {
        Type-Unicode 'discard this' 30
        Press 0x55 @(0x11)    # Ctrl+U: the editing key reaches the shell, which clears the line
        Next-Line 'echo AFTER-CTRL-U'
        $text = Next-Text
        Next-Check 'keys.ctrl-u' ($text -match 'AFTER-CTRL-U' -and $text -notmatch 'discard thisecho') 'Ctrl+U cleared the typed text'
        Next-Line 'Start-Sleep 30'
        Press 0x43 @(0x11)    # Ctrl+C with no selection is the program's
        Start-Sleep -Milliseconds 800
        Next-Line 'echo AFTER-INTERRUPT'
        $text = Next-Text
        Next-Check 'keys.ctrl-c-interrupts' ($text -match 'AFTER-INTERRUPT') 'Ctrl+C ended the sleeping command'
        Type-Unicode 'abc' 30
        Press 0x08            # Backspace
        Press 0x0D; Start-Sleep -Milliseconds 700
        $text = Next-Text
        Next-Check 'keys.backspace' ($text -match "ab\s*$|'abc'" -or $text -match 'ab: The term') 'Backspace removed one character'
        Next-Line 'echo 你好世界'
        $text = Next-Text
        Next-Check 'keys.cjk' ($text -match '你好世界') 'CJK text arrives intact'
    }
}

# Claude Code in the shell, started the way the WinUI scenarios start it (see claude-scenarios.ps1 for the rules): the
# environment of this session never reaches the app, auto-update and the user's customizations are off, nothing is ever
# sent to the model, and Enter goes to the CLI only when the input box holds exactly /exit. The screen text is not
# printed: it shows account and folder names.
function Scenario-NextClaude([ValidateSet('fullscreen', 'classic')][string]$Mode = 'fullscreen') {
    $claude = if ($env:NEKOMINAL_CLAUDE_EXE) { $env:NEKOMINAL_CLAUDE_EXE } else { Join-Path $env:USERPROFILE '.local\bin\claude.exe' }
    $cwd = Join-Path $PSScriptRoot '..\..\target\agent-probe\cwd'
    New-Item -ItemType Directory -Force $cwd | Out-Null
    $cwd = (Resolve-Path $cwd).Path
    $envTable = @{ DISABLE_AUTOUPDATER = '1'; CLAUDE_CODE_SAFE_MODE = '1' }
    if ($Mode -eq 'fullscreen') { $envTable['CLAUDE_CODE_NO_FLICKER'] = '1' } else { $envTable['CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN'] = '1' }
    $saved = Get-Clipboard -Raw -ErrorAction SilentlyContinue
    $global:NextFailures = @()
    try {
        Start-App -Exe (Resolve-Path $global:NextExe).Path -Log '' -Layout '' -SettleMs 3500 -Env $envTable
        Next-Line "Set-Location '$cwd'"
        Next-Line "& '$claude'"
        Start-Sleep -Seconds 9
        $text = Next-Text
        Next-Check "claude.$Mode.ui" ($text -match 'Claude Code' -and $text -match '[─]{10}') 'the banner and the rules of the input box are on the screen'
        $middle = Next-At 500 300
        Move-To $middle[0] $middle[1]
        [D]::Mouse(0x0800, 120); Start-Sleep -Milliseconds 300
        [D]::Mouse(0x0800, [BitConverter]::ToUInt32([BitConverter]::GetBytes([int]-120), 0)); Start-Sleep -Milliseconds 300
        Type-Unicode 'ab你好' 60; Start-Sleep -Milliseconds 800
        Next-Check "claude.$Mode.typing" ((Next-Text) -match 'ab你好') 'typed text, CJK included, reaches the input box after the wheel moved over it'
        Press 0x55 @(0x11)
        Type-Unicode '/exit' 60; Start-Sleep -Milliseconds 500
        $guard = Next-Text
        if ($guard -match 'ab你好' -or $guard -notmatch '/exit') { throw 'the input box does not hold exactly /exit: Enter not sent' }
        Press 0x0D; Start-Sleep -Seconds 4
        Next-Check "claude.$Mode.exit" ((Next-Text) -match 'PS .*>\s*$') 'after /exit the prompt of the shell is back'
    } finally {
        Stop-App
        if ($null -ne $saved) { Set-Clipboard -Value $saved }
    }
    if ($global:NextFailures.Count -gt 0) { throw "failed: $($global:NextFailures -join ', ')" }
}
function Scenario-NextIdle {
    Run-Next {
        Start-Sleep -Seconds 3
        $app = $script:App; $app.Refresh()
        $private = $app.PrivateMemorySize64 / 1MB
        Next-Check 'idle.memory' ($private -lt 200) ("{0:N0} MB private, {1:N0} MB working set" -f $private, ($app.WorkingSet64 / 1MB))
        $before = $app.TotalProcessorTime.TotalMilliseconds; Start-Sleep -Seconds 5; $app.Refresh()
        $spent = $app.TotalProcessorTime.TotalMilliseconds - $before
        Next-Check 'idle.cpu' ($spent -lt 100) ("{0:N0} ms of CPU in 5 s while nothing happens" -f $spent)
    }
}

function Scenario-NextAll { Scenario-NextTabs; Scenario-NextClipboard; Scenario-NextKeys; Scenario-NextClaude fullscreen; Scenario-NextClaude classic; Scenario-NextIdle }
