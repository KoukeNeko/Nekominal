param([int]$Lines = 3000)
# Produces a burst of output with colour, CJK, OSC 7 / 133 / 7501, a synchronized-update frame and a
# query, to exercise the pane host end to end.
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$e = [string][char]27
$bel = [string][char]7
[Console]::Write("$e]7;file:///C:/Users/me/project$e\")
[Console]::Write("$e]133;A$bel")
for ($i = 1; $i -le $Lines; $i++) {
  [Console]::Write("$e[3$($i % 7 + 1)mline $i 你好，世界$e[0m`r`n")
}
[Console]::Write("$e[?2026h$e[2Jframe-inside-sync$e[?2026l")
[Console]::Write("$e]7501;state=done:id=burst:app=test$e\")
[Console]::Write("$e]133;D;0$bel")
[Console]::Write("`r`nBURST-DONE`r`n")
