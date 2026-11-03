param([Parameter(Mandatory)][ValidateSet('ore-seed','ore-mine','claim-add','add','dig','flatten','paste','undo','verify','compare','save','race','status','asymmetric','shape-baseline','shape-verify','mip-baseline','mip-verify','probe-player')][string]$Command,[int]$TimeoutSeconds=45,[string]$ServerDirectory=(Join-Path $PSScriptRoot 'test-server'))
$ErrorActionPreference='Stop'
$serverDir=(Resolve-Path -LiteralPath $ServerDirectory).Path
$result=Join-Path $serverDir 'terrain-test-result.txt'
if (Test-Path -LiteralPath $result) {Remove-Item -LiteralPath $result}
"$Command`n#$([guid]::NewGuid())" | Set-Content (Join-Path $serverDir 'terrain-test-command.txt')
$serverPid=[int](Get-Content (Join-Path $serverDir 'test-server.pid'))
$deadline=(Get-Date).AddSeconds($TimeoutSeconds)
while ((Get-Date) -lt $deadline) {
    if (!(Get-Process -Id $serverPid -ErrorAction SilentlyContinue)) {throw 'Test server exited'}
    if (Test-Path -LiteralPath $result) {
        $text=Get-Content -LiteralPath $result
        if ($Command -eq 'race' -and $text -match 'race verified') {$text;return}
        if ($Command -ne 'race' -and ($Command -ne 'verify' -or $text -match 'different=0 unloaded=0')) {$text;return}
    }
    Start-Sleep -Milliseconds 500
}
throw "Test command timed out: $Command"
