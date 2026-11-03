param([switch]$ProbeOnly,[string]$ServerDirectory=(Join-Path $PSScriptRoot 'test-server'))
$ErrorActionPreference='Stop'
$serverDir=(Resolve-Path -LiteralPath $ServerDirectory).Path
$pidFile=Join-Path $serverDir 'test-server.pid'
if (Test-Path -LiteralPath $pidFile) {
    $old=Get-Process -Id ([int](Get-Content $pidFile)) -ErrorAction SilentlyContinue
    if ($old -and $old.Path -eq (Join-Path $serverDir 'enshrouded_server.exe')) {throw 'Test server already running'}
}
$env:TERRAIN_TEST_PROBE_ONLY = if ($ProbeOnly) {'1'} else {'0'}
try {
    $p=Start-Process -FilePath (Join-Path $serverDir 'enshrouded_server.exe') -WorkingDirectory $serverDir -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $serverDir 'current-stdout.log') -RedirectStandardError (Join-Path $serverDir 'current-stderr.log')
} finally { Remove-Item Env:TERRAIN_TEST_PROBE_ONLY }
$p.Id | Set-Content $pidFile
"Test server PID: $($p.Id)"
