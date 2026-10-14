# Writes the control sequences tests/selftest.rs expects, in two phases. Windows PowerShell 5.1 compatible and
# ASCII only: the wide characters are built from code points.
$e = [char]27
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding $false
$o = [Console]::Out
function Send([string]$text) { $o.Write($text); $o.Flush() }

# Phase 1: alternate screen, button mouse tracking with SGR encoding, bracketed paste, focus reports and a
# kitty keyboard push; a title and a program status report; a synchronized frame; wide text.
Send "$e[?1049h$e[?1002h$e[?1006h$e[?2004h$e[?1004h$e[>5u"
Send "$e]0;selftest-title$e\"
Send "$e]7501;state=working:app=selftest$e\"
Send "$e[?2026h$e[H$e[2JPHASE-1$e[?2026l"
$wide = [string][char]0x5BBD + [string][char]0x5B57
Send "$e[2;1H$wide end"
# Queries the harness must answer: DA1, XTVERSION, kitty keyboard flags, DECRQM for synchronized output.
Send "$e[c$e[>q$e[?u$e[?2026`$p"
Start-Sleep -Seconds 3

# Phase 2: everything off again, attributes reset.
Send "$e[<u$e[?1004l$e[?2004l$e[?1006l$e[?1002l$e[?1049l"
Send "PHASE-2$e[0m`r`n"
Start-Sleep -Milliseconds 800
