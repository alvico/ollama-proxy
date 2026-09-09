<#
    Starts the tuned ollama backend(s) plus the routing proxy.

    The proxy takes 11434 so existing clients need no reconfiguration; the
    backends move to loopback-only ports behind it. Sleep is suppressed for as
    long as this script runs (see ollama-serve-awake.ps1 for the mechanism).

    Only the f16 backend starts by default. The q8_0 one exists so a model can
    be measured on both KV cache types, but nothing is routed to it: every model
    measured so far was faster on f16, and q8_0 did not even reproduce between
    runs (see _routes and _q8_instability in config.json). Starting it anyway
    would cost a process and six seconds for a backend no request reaches, so it
    is opt-in. Pass -WithQ8 before running bench-model.ps1, which needs both
    backends up to compare them.
#>
param(
    [switch]$WithQ8,
    [string]$ProxyDir    = $PSScriptRoot,
    [string]$OllamaDir   = $env:OLLAMA_DIR,
    [string]$ServeScript = (Join-Path $ProxyDir "ollama-serve-awake.ps1"),
    [int]$Q8Port         = 11435,
    [int]$F16Port        = 11436,
    [string]$LogDir      = (Join-Path $env:LOCALAPPDATA "Ollama\serve-logs")
)

# Where ollama lives is a property of the host, not of this project, so it is
# not hardcoded: set OLLAMA_DIR, or let it be resolved from whatever ollama is
# already on PATH.
if (-not $OllamaDir) {
    $onPath = Get-Command ollama.exe -ErrorAction SilentlyContinue
    if ($onPath) {
        $OllamaDir = Split-Path $onPath.Source -Parent
    } else {
        Write-Host "cannot find ollama.exe: set OLLAMA_DIR or pass -OllamaDir" -ForegroundColor Red
        exit 1
    }
}

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

# Backends bind to loopback only: the proxy is the only listener reachable from
# anywhere else.
if ($WithQ8) {
    Write-Host "Starting q8_0 backend on 127.0.0.1:$Q8Port ..." -ForegroundColor Cyan
    $procs += Start-Process powershell -PassThru -WindowStyle Hidden -ArgumentList @(
        '-ExecutionPolicy','Bypass','-File',$ServeScript,
        '-SkipExistingCheck','-Port',$Q8Port,'-KVCacheType','q8_0','-OllamaDir',$OllamaDir,'-LogDir',$LogDir)

    Start-Sleep -Seconds 6
} else {
    Write-Host "Skipping q8_0 backend (nothing routes to it; -WithQ8 to measure against it)." -ForegroundColor DarkGray
}

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
