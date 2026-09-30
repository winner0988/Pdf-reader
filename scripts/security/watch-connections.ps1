<#
.SYNOPSIS
  Logs every network endpoint opened by the PDF Reader processes while you use the app.

.DESCRIPTION
  Polls TCP connections and UDP endpoints owned by pdf-reader.exe, pdf_worker.exe and the
  WebView2 processes they start (msedgewebview2.exe children), and prints any remote endpoint
  it sees. Expected output during the offline verification: nothing but the header.

  Polling can miss connections shorter than the interval, so this complements (does not
  replace) the Process Monitor procedure in docs/security/offline-verification.md. TCP is read
  with netstat, several times faster than Get-NetTCPConnection, and the process tree only every
  second, so a poll takes about a tenth of a second: the update check's one connection (#64),
  which lasts about a second, is seen.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts/security/watch-connections.ps1 -Seconds 120
#>
param(
  [int]$Seconds = 120,
  [int]$IntervalMs = 50,
  [string[]]$ProcessNames = @('pdf-reader', 'pdf_worker')
)

$names = $ProcessNames

# TCP connections with a remote end: Remote (address:port), State and OwningProcess.
function Get-TcpConnections {
  foreach ($protocol in 'TCP', 'TCPv6') {
    foreach ($line in (netstat -ano -p $protocol)) {
      $fields = -split $line
      if ($fields.Count -eq 5 -and $fields[0] -eq 'TCP' -and $fields[2] -notmatch '^(0\.0\.0\.0|\[::\]):0$') {
        [pscustomobject]@{ Remote = $fields[2]; State = $fields[3]; OwningProcess = [int]$fields[4] }
      }
    }
  }
}

function Get-AppProcessIds {
  $roots = Get-Process -Name $names -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Id
  $ids = [System.Collections.Generic.HashSet[int]]::new()
  foreach ($id in $roots) { [void]$ids.Add($id) }
  # Add descendants (WebView2 runs in msedgewebview2.exe children of the app).
  $all = Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId
  do {
    $added = 0
    foreach ($p in $all) {
      if ($ids.Contains([int]$p.ParentProcessId) -and $ids.Add([int]$p.ProcessId)) { $added++ }
    }
  } while ($added -gt 0)
  return $ids
}

Write-Host "Watching network endpoints of $($names -join ', ') and child processes for $Seconds s..."
$seen = [System.Collections.Generic.HashSet[string]]::new()
$deadline = (Get-Date).AddSeconds($Seconds)
$ids = $null
$treeRead = [datetime]::MinValue
while ((Get-Date) -lt $deadline) {
  if (((Get-Date) - $treeRead).TotalSeconds -ge 1) {
    $ids = Get-AppProcessIds
    $treeRead = Get-Date
  }
  if ($ids.Count -gt 0) {
    $tcp = Get-TcpConnections | Where-Object { $ids.Contains($_.OwningProcess) }
    foreach ($c in $tcp) {
      $key = "TCP $($c.OwningProcess) $($c.Remote)"
      if ($seen.Add($key)) {
        $name = (Get-Process -Id $c.OwningProcess -ErrorAction SilentlyContinue).ProcessName
        Write-Host "$(Get-Date -Format HH:mm:ss.fff) TCP $name ($($c.OwningProcess)) -> $($c.Remote) [$($c.State)]"
      }
    }
    $udp = Get-NetUDPEndpoint -ErrorAction SilentlyContinue | Where-Object { $ids.Contains([int]$_.OwningProcess) }
    foreach ($u in $udp) {
      $key = "UDP $($u.OwningProcess) $($u.LocalAddress):$($u.LocalPort)"
      if ($seen.Add($key)) {
        $name = (Get-Process -Id $u.OwningProcess -ErrorAction SilentlyContinue).ProcessName
        Write-Host "$(Get-Date -Format HH:mm:ss.fff) UDP $name ($($u.OwningProcess)) bound $($u.LocalAddress):$($u.LocalPort)"
      }
    }
  }
  Start-Sleep -Milliseconds $IntervalMs
}
Write-Host "Done. Endpoints seen: $($seen.Count)"
