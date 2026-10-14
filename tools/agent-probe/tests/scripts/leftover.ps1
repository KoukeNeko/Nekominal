# Leaves bold red set and exits: the probe must report the attributes as left over.
$e = [char]27
$o = [Console]::Out
$o.Write("$e[1;31mx")
$o.Flush()
Start-Sleep -Milliseconds 500
