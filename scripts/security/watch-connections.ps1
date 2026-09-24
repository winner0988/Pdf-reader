<#
.SYNOPSIS
  Logs every network endpoint opened by the PDF Reader processes while you use the app.

.DESCRIPTION
  Polls TCP connections and UDP endpoints owned by pdf-reader.exe, pdf_worker.exe and the
  WebView2 processes they start (msedgewebview2.exe children), and prints any remote endpoint
  it sees. Expected output during the offline verification: nothing but the header.

  Polling can miss connections shorter than the interval, so this complements (does not
  replace) the Process Monitor procedure in docs/security/offline-verification.md.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts/security/watch-connections.ps1 -Seconds 120
#>
param(
  [int]$Seconds = 120,
  [int]$IntervalMs = 200,
  [string[]]$ProcessNames = @('pdf-reader', 'pdf_worker')
)

$names = $ProcessNames

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
while ((Get-Date) -lt $deadline) {
  $ids = Get-AppProcessIds
  if ($ids.Count -gt 0) {
    $tcp = Get-NetTCPConnection -ErrorAction SilentlyContinue |
      Where-Object { $ids.Contains([int]$_.OwningProcess) -and $_.RemoteAddress -notin @('0.0.0.0', '::') }
    foreach ($c in $tcp) {
      $key = "TCP $($c.OwningProcess) $($c.RemoteAddress):$($c.RemotePort)"
      if ($seen.Add($key)) {
        $name = (Get-Process -Id $c.OwningProcess -ErrorAction SilentlyContinue).ProcessName
        Write-Host "$(Get-Date -Format HH:mm:ss.fff) TCP $name ($($c.OwningProcess)) -> $($c.RemoteAddress):$($c.RemotePort) [$($c.State)]"
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
