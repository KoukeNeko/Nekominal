param([string]$ProbeFile)
# Writes each probe sequence (given as hex) between two plain-text markers so the host can tell what
# ConPTY forwarded for it.
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
foreach ($line in [System.IO.File]::ReadAllLines($ProbeFile)) {
  $parts = $line.Split("`t")
  $tag = $parts[0]; $hex = $parts[1]
  $bytes = [byte[]]::new($hex.Length / 2)
  for ($i = 0; $i -lt $bytes.Length; $i++) { $bytes[$i] = [Convert]::ToByte($hex.Substring(2 * $i, 2), 16) }
  $text = [System.Text.Encoding]::UTF8.GetString($bytes)
  [Console]::Write("<<B:$tag>>")
  [Console]::Write($text)
  [Console]::Write("<<E:$tag>>`r`n")
  Start-Sleep -Milliseconds 40
}
Start-Sleep -Milliseconds 400
