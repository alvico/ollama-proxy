<#
    Starts the two tuned ollama backends plus the routing proxy.

    The proxy takes 11434 so existing clients need no reconfiguration; the
    backends move to loopback-only ports behind it. Sleep is suppressed for as
    long as this script runs (see ollama-serve-awake.ps1 for the mechanism).
#>
param(
    [string]$OllamaDir   = "E:\claude\Ollama",
    [string]$ProxyDir    = "F:\Code\work\ollama-proxy",
    [string]$ServeScript = (Join-Path $ProxyDir "ollama-serve-awake.ps1"),
    [int]$Q8Port         = 11435,
    [int]$F16Port        = 11436,
    [string]$LogDir      = (Join-Path $env:LOCALAPPDATA "Ollama\serve-logs")
)

$proxyExe = Join-Path $ProxyDir "target\release\ollama-proxy.exe"
$config   = Join-Path $ProxyDir "config.json"

foreach ($p in @($proxyExe, $config, $ServeScript)) {
    if (-not (Test-Path $p)) {
        Write-Host "missing: $p" -ForegroundColor Red
        if ($p -eq $proxyExe) { Write-Host "build it first: cargo build --release" -ForegroundColor Yellow }
        exit 1
    }
}

$code = @'
using System.Runtime.InteropServices;
public static class PowerKeeper {
    [DllImport("kernel32.dll")]
    static extern uint SetThreadExecutionState(uint esFlags);
    const uint ES_CONTINUOUS      = 0x80000000;
    const uint ES_SYSTEM_REQUIRED = 0x00000001;
    public static uint KeepAwake() { return SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED); }
    public static uint Restore()   { return SetThreadExecutionState(ES_CONTINUOUS); }
}
'@
Add-Type -TypeDefinition $code

Write-Host "Stopping existing Ollama processes so the new configs apply..."
# llama-server.exe is spawned by ollama serve but is not killed with it: an
# orphaned runner keeps its model's weights in VRAM and shows up as a second
# server alongside the stack's own.
Get-Process -Name "ollama app", "ollama", "llama-server" -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 2

if ([PowerKeeper]::KeepAwake() -eq 0) {
    Write-Host "Failed to suppress sleep." -ForegroundColor Red
    exit 1
}

$procs = @()

# Backends bind to loopback only: the proxy is the sole LAN-facing listener.
Write-Host "Starting q8_0 backend on 127.0.0.1:$Q8Port ..." -ForegroundColor Cyan
$procs += Start-Process powershell -PassThru -WindowStyle Hidden -ArgumentList @(
    '-ExecutionPolicy','Bypass','-File',$ServeScript,
    '-SkipExistingCheck','-Port',$Q8Port,'-KVCacheType','q8_0','-OllamaDir',$OllamaDir,'-LogDir',$LogDir)

Start-Sleep -Seconds 6

Write-Host "Starting f16 backend on 127.0.0.1:$F16Port ..." -ForegroundColor Cyan
$procs += Start-Process powershell -PassThru -WindowStyle Hidden -ArgumentList @(
    '-ExecutionPolicy','Bypass','-File',$ServeScript,
    '-SkipExistingCheck','-Port',$F16Port,'-KVCacheType','f16','-OllamaDir',$OllamaDir,'-LogDir',$LogDir)

Start-Sleep -Seconds 6

$stamp     = Get-Date -Format "yyyyMMdd-HHmmss"
$proxyLog  = Join-Path $LogDir "proxy-$stamp.log"
New-Item -ItemType Directory -Force -Path $LogDir | Out-Null

Write-Host "Starting proxy on 11434 ..." -ForegroundColor Cyan
Write-Host "  proxy log: $proxyLog"
$proxy = Start-Process -FilePath $proxyExe -ArgumentList $config -WorkingDirectory $ProxyDir `
    -PassThru -WindowStyle Hidden -RedirectStandardOutput "$proxyLog" -RedirectStandardError "$proxyLog.err"
$procs += $proxy

Write-Host ""
Write-Host "Stack up. Point clients at http://127.0.0.1:11434 (or the LAN address)." -ForegroundColor Green
Write-Host "Press Ctrl+C to stop everything and restore sleep behavior."

try {
    while (-not $proxy.HasExited) {
        Start-Sleep -Seconds 5
        [PowerKeeper]::KeepAwake() | Out-Null
    }
    Write-Host "Proxy exited (code $($proxy.ExitCode)). See $proxyLog.err" -ForegroundColor Red
}
finally {
    foreach ($p in $procs) {
        if ($p -and -not $p.HasExited) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue }
    }
    # The backend launcher scripts spawn ollama.exe as children, which spawn
    # llama-server.exe runners that survive their parent; clear both.
    Get-Process -Name "ollama", "llama-server" -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    [PowerKeeper]::Restore() | Out-Null
    Write-Host "Stack stopped; sleep behavior restored."
}
