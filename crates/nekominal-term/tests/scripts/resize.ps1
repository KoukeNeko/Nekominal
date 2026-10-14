# Output that wraps, a pause in which the host resizes the terminal twice, and markers after each pause.
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$long = ('0123456789' * 13)
[Console]::Write("first line`r`n$long`r`nlast line before resize`r`n<<PHASE1>>")
Start-Sleep -Milliseconds 2000
[Console]::Write("`r`n<<AFTER-RESIZE-1>>`r`n")
Start-Sleep -Milliseconds 2000
[Console]::Write("`r`n<<AFTER-RESIZE-2>>`r`n")
Start-Sleep -Milliseconds 400
