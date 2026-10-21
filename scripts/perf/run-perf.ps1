$ErrorActionPreference = "Stop"
$port = 9240
$app = "C:\Users\UserB\AppData\Local\Temp\target-240\release\pdf-reader.exe"
$doc1 = "tests/corpus/large/output/large-1000-pages.pdf"
$docA0 = "tests/corpus/large/output/large-a0-poster.pdf"

$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=$port"
$env:WEBVIEW2_USER_DATA_FOLDER = "C:\Users\UserB\AppData\Local\Temp\wv2-perf-240"
$env:PDF_READER_DATA_DIR = "C:\Users\UserB\AppData\Local\Temp\appdata-perf-240"

Remove-Item -Recurse -Force $env:WEBVIEW2_USER_DATA_FOLDER -ErrorAction SilentlyContinue
Remove-Item -Recurse -Force $env:PDF_READER_DATA_DIR -ErrorAction SilentlyContinue

Write-Host "Starting app..."
$proc = Start-Process -FilePath $app -ArgumentList $doc1 -PassThru

try {
  $ready = $false
  for ($i = 0; $i -lt 60; $i++) {
    Start-Sleep -Milliseconds 500
    try {
      $resp = Invoke-RestMethod -Uri "http://127.0.0.1:$port/json/version" -TimeoutSec 1
      if ($resp) { $ready = $true; break }
    } catch {}
  }
  if (-not $ready) {
    throw "App did not start listening on port $port within 30 seconds"
  }
  Write-Host "App listening on port $port, running viewer-perf.mjs..."
  node scripts/perf/viewer-perf.mjs $port test-results/perf $app $docA0
} finally {
  Write-Host "Cleaning up app process $($proc.Id)..."
  taskkill /PID $proc.Id /T /F 2>$null
  Start-Sleep -Milliseconds 500
  Remove-Item -Recurse -Force $env:WEBVIEW2_USER_DATA_FOLDER -ErrorAction SilentlyContinue
  Remove-Item -Recurse -Force $env:PDF_READER_DATA_DIR -ErrorAction SilentlyContinue
}
