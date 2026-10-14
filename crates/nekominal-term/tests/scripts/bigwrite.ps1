param([int]$MiB = 16, [int]$ChunkChars = 32768)
# Writes $MiB megabytes of coloured lines in large WriteConsole calls.
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$e = [string][char]27
$line = "$e[38;5;196mred$e[0m $e[32mgreen$e[0m plain text for the terminal emulator throughput test 0123456789`r`n"
$sb = [System.Text.StringBuilder]::new()
while ($sb.Length -lt $ChunkChars) { [void]$sb.Append($line) }
$chunk = $sb.ToString()
$count = [int](($MiB * 1MB) / $chunk.Length)
for ($i = 0; $i -lt $count; $i++) { [Console]::Write($chunk) }
