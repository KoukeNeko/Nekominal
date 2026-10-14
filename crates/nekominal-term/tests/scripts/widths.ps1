param([string]$CasesFile, [string]$OutFile)
# For every case (label TAB space-separated hex code points): home the cursor, erase the line, print the
# text and record where ConPTY's own console buffer put the cursor. A title marker with the same
# numbers goes into the byte stream so the host can sample its emulator cursor at the same moment.
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$e = [string][char]27
$bel = [string][char]7
$log = [System.Collections.Generic.List[string]]::new()
$index = 0
foreach ($line in [System.IO.File]::ReadAllLines($CasesFile)) {
  $parts = $line.Split("`t")
  $text = -join ($parts[1].Split(' ') | ForEach-Object { [char]::ConvertFromUtf32([Convert]::ToInt32($_, 16)) })
  [Console]::Write("$e[1;1H$e[2K$text")
  $left = [Console]::CursorLeft
  $top = [Console]::CursorTop
  [Console]::Write("$e]2;W$index;$left;$top$bel")
  $log.Add($parts[0] + "`t" + $left + "`t" + $top)
  $index++
}
Start-Sleep -Milliseconds 300
[System.IO.File]::WriteAllLines($OutFile, $log)
