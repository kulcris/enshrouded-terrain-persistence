param([string]$ServerDirectory=(Join-Path $PSScriptRoot 'test-server'))
$ErrorActionPreference='Stop'
$serverDir=(Resolve-Path -LiteralPath $ServerDirectory).Path
$pidFile=Join-Path $serverDir 'test-server.pid'
if (!(Test-Path -LiteralPath $pidFile)) { return }
$serverPid=[int](Get-Content -LiteralPath $pidFile)
$p=Get-Process -Id $serverPid -ErrorAction SilentlyContinue
if (!$p) { return }
if ($p.Path -ne (Join-Path $serverDir 'enshrouded_server.exe')) { throw 'PID belongs to another process' }
Add-Type -TypeDefinition 'using System; using System.Runtime.InteropServices; public class TerrainTestConsole { public delegate bool Handler(uint c); static Handler handler = c => true; public static IntPtr HandlerPointer = Marshal.GetFunctionPointerForDelegate(handler); [DllImport("kernel32.dll")] public static extern bool FreeConsole(); [DllImport("kernel32.dll")] public static extern bool AttachConsole(uint pid); [DllImport("kernel32.dll")] public static extern bool SetConsoleCtrlHandler(IntPtr h, bool add); [DllImport("kernel32.dll")] public static extern bool GenerateConsoleCtrlEvent(uint e, uint g); }'
[TerrainTestConsole]::FreeConsole() | Out-Null
if (![TerrainTestConsole]::AttachConsole($serverPid)) {throw 'Cannot attach to test server console'}
[TerrainTestConsole]::SetConsoleCtrlHandler([IntPtr]::Zero,$true) | Out-Null
[TerrainTestConsole]::SetConsoleCtrlHandler([TerrainTestConsole]::HandlerPointer,$true) | Out-Null
if (![TerrainTestConsole]::GenerateConsoleCtrlEvent(1,0)) {throw 'Cannot request graceful server shutdown'}
if (!$p.WaitForExit(20000)) {throw 'Server has not exited after shutdown request'}
'Test server stopped gracefully.'
[TerrainTestConsole]::FreeConsole() | Out-Null
[TerrainTestConsole]::SetConsoleCtrlHandler([TerrainTestConsole]::HandlerPointer,$false) | Out-Null
[TerrainTestConsole]::SetConsoleCtrlHandler([IntPtr]::Zero,$false) | Out-Null
