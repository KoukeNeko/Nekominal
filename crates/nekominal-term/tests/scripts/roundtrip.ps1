param([string]$ProbeFile, [string]$OutFile)
# Child side of the query round trip: sends each query (hex in $ProbeFile), then reads whatever the
# terminal answers on stdin with ENABLE_VIRTUAL_TERMINAL_INPUT set, and logs it. Afterwards it logs
# the raw input chunks the host injects (keys, paste, focus) to show how ConPTY delivers them.
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
Add-Type -Namespace K -Name Con -MemberDefinition @'
[DllImport("kernel32.dll")] public static extern System.IntPtr GetStdHandle(int n);
[DllImport("kernel32.dll")] public static extern bool GetConsoleMode(System.IntPtr h, out uint m);
[DllImport("kernel32.dll")] public static extern bool SetConsoleMode(System.IntPtr h, uint m);
'@
$h = [K.Con]::GetStdHandle(-10)
[uint32]$old = 0
[void][K.Con]::GetConsoleMode($h, [ref]$old)
[void][K.Con]::SetConsoleMode($h, 0x200)
$stream = [Console]::OpenStandardInput()
$script:buf = [byte[]]::new(4096)
$script:pending = $null

function Read-Input([int]$quietMs, [int]$maxMs) {
  $acc = [System.Collections.Generic.List[byte]]::new()
  $stop = [DateTime]::UtcNow.AddMilliseconds($maxMs)
  $lastData = $null
  while ([DateTime]::UtcNow -lt $stop) {
    if ($null -eq $script:pending) { $script:pending = $stream.ReadAsync($script:buf, 0, $script:buf.Length) }
    if ($script:pending.Wait(20)) {
      $n = $script:pending.Result
      for ($i = 0; $i -lt $n; $i++) { $acc.Add($script:buf[$i]) }
      $script:pending = $null
      $lastData = [DateTime]::UtcNow
    }
    if ($null -ne $lastData -and ([DateTime]::UtcNow - $lastData).TotalMilliseconds -gt $quietMs) { break }
  }
  return , $acc.ToArray()
}

function Escape([byte[]]$bytes) {
  $sb = [System.Text.StringBuilder]::new()
  foreach ($b in $bytes) {
    if ($b -eq 27) { [void]$sb.Append('\e') }
    elseif ($b -ge 32 -and $b -lt 127) { [void]$sb.Append([char]$b) }
    else { [void]$sb.Append(('\x{0:x2}' -f $b)) }
  }
  return $sb.ToString()
}

$log = [System.Collections.Generic.List[string]]::new()
foreach ($line in [System.IO.File]::ReadAllLines($ProbeFile)) {
  $parts = $line.Split("`t")
  $hex = $parts[1]
  $bytes = [byte[]]::new($hex.Length / 2)
  for ($i = 0; $i -lt $bytes.Length; $i++) { $bytes[$i] = [Convert]::ToByte($hex.Substring(2 * $i, 2), 16) }
  [Console]::Write([System.Text.Encoding]::UTF8.GetString($bytes))
  $answer = Read-Input 120 700
  $log.Add("Q:" + $parts[0] + "`t" + (Escape $answer))
}
[Console]::Write("<<INPUT-READY>>")
$n = 0
while ($true) {
  $chunk = Read-Input 120 8000
  if ($chunk.Length -eq 0) { break }
  $log.Add("I:" + $n + "`t" + (Escape $chunk))
  $n++
  if ($chunk[$chunk.Length - 1] -eq [byte][char]'q') { break }
}
[void][K.Con]::SetConsoleMode($h, $old)
[System.IO.File]::WriteAllLines($OutFile, $log)
