param(
    [string]$OllamaDir = "E:\claude\Ollama",
    [int]$ContextLength = 16384,
    # Single user, so one slot. Every extra slot multiplies the KV cache by its
    # count, and on a 10GB card that KV is what pushes layers off the GPU:
    # gemma4 went 44/49 -> 49/49 layers and 28 -> 63 tok/s once it stopped
    # paying for a second slot it never used.
    [int]$NumParallel = 1,
    [string]$KeepAlive = "30m",
    [int]$FlashAttention = 1,
    # f16, not q8_0: q8_0 quantises in blocks of 32, which must divide the model's
    # V head dimension. kimi-linear has n_embd_head_v=72 and fails to load on q8_0.
    # Pass -KVCacheType q8_0 to halve KV memory if you only run qwen/gemma.
    [string]$KVCacheType = "f16",
    [int]$MaxLoadedModels = 1,
    [int]$Port = 11434,
    # 127.0.0.1 = this machine only. Use 0.0.0.0 to serve other machines on the LAN.
    [string]$BindAddress = "127.0.0.1",
    [string]$LogDir = (Join-Path $env:LOCALAPPDATA "Ollama\serve-logs"),
    [switch]$SkipExistingCheck
)

$ollamaExe = Join-Path $OllamaDir "ollama.exe"
if (-not (Test-Path $ollamaExe)) {
    Write-Host "ollama.exe not found at $ollamaExe" -ForegroundColor Red
    exit 1
}

$code = @'
using System.Runtime.InteropServices;
public static class PowerKeeper {
    [DllImport("kernel32.dll")]
    static extern uint SetThreadExecutionState(uint esFlags);

    const uint ES_CONTINUOUS      = 0x80000000;
    const uint ES_SYSTEM_REQUIRED = 0x00000001;

    // Flags live in C#: PowerShell parses 0x80000001 as a negative Int32,
    // which fails to convert to the uint parameter and silently does nothing.
    public static uint KeepAwake() {
        return SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED);
    }
    public static uint Restore() {
        return SetThreadExecutionState(ES_CONTINUOUS);
    }
}
'@
Add-Type -TypeDefinition $code

function Get-ListeningPid {
    return Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue |
        Select-Object -First 1 -ExpandProperty OwningProcess
}

function Test-ServerListening {
    return [bool](Get-ListeningPid)
}

$env:OLLAMA_HOST = "${BindAddress}:$Port"
$env:OLLAMA_CONTEXT_LENGTH = "$ContextLength"
$env:OLLAMA_NUM_PARALLEL = "$NumParallel"
$env:OLLAMA_KEEP_ALIVE = $KeepAlive
$env:OLLAMA_FLASH_ATTENTION = "$FlashAttention"
$env:OLLAMA_KV_CACHE_TYPE = $KVCacheType
$env:OLLAMA_MAX_LOADED_MODELS = "$MaxLoadedModels"

$isLanBind = $BindAddress -notin @("127.0.0.1", "localhost", "::1")

if ($isLanBind) {
    # Browser-based clients send an Origin header that ollama rejects by default.
    $env:OLLAMA_ORIGINS = "*"

    $lanIPs = Get-NetIPAddress -AddressFamily IPv4 -ErrorAction SilentlyContinue |
        Where-Object { $_.IPAddress -notlike '127.*' -and $_.IPAddress -notlike '169.254.*' } |
        Select-Object -ExpandProperty IPAddress

    Write-Host ""
    Write-Host "Serving on the network. Reachable from other machines at:" -ForegroundColor Cyan
    foreach ($ip in $lanIPs) { Write-Host "  http://${ip}:$Port" -ForegroundColor Cyan }
    Write-Host "The Ollama API is unauthenticated: anyone who can reach this port can run, pull and delete models." -ForegroundColor Yellow

    # A bound socket is useless if the firewall drops the inbound packets.
    $fwRule = Get-NetFirewallPortFilter -ErrorAction SilentlyContinue |
        Where-Object { $_.LocalPort -eq $Port } |
        ForEach-Object { Get-NetFirewallRule -AssociatedNetFirewallPortFilter $_ -ErrorAction SilentlyContinue } |
        Where-Object { $_.Direction -eq 'Inbound' -and $_.Enabled -eq 'True' -and $_.Action -eq 'Allow' }

    if (-not $fwRule) {
        Write-Host "WARNING: no inbound firewall rule allows port $Port; remote clients will time out." -ForegroundColor Red
        Write-Host "Run once in an ELEVATED PowerShell. RemoteAddress LocalSubnet is what keeps this" -ForegroundColor Red
        Write-Host "LAN-only; do not widen it to Any:" -ForegroundColor Red
        Write-Host "  New-NetFirewallRule -DisplayName 'Ollama serve (LAN)' -Direction Inbound -Protocol TCP -LocalPort $Port -RemoteAddress LocalSubnet -Profile Any -Action Allow" -ForegroundColor Red
    }
    else {
        # An existing rule may be scoped wider than the local subnet.
        $wideOpen = $fwRule | ForEach-Object { Get-NetFirewallAddressFilter -AssociatedNetFirewallRule $_ -ErrorAction SilentlyContinue } |
            Where-Object { $_.RemoteAddress -contains 'Any' }
        if ($wideOpen) {
            Write-Host "WARNING: an existing rule for port $Port allows ANY remote address, not just the LAN." -ForegroundColor Red
        }
    }
    Write-Host ""
}

$server = $null
$startedByUs = $true
$errLog = $null

if ($SkipExistingCheck) {
    # Leave any running server alone. If one is already serving our port, attach
    # to it and just hold the keep-awake request for as long as it lives.
    $existingPid = Get-ListeningPid
    if ($existingPid) {
        $server = Get-Process -Id $existingPid -ErrorAction SilentlyContinue
    }
    if ($server) {
        $startedByUs = $false
        Write-Host "Port $Port is already served by $($server.ProcessName) (PID $($server.Id)); attaching to it." -ForegroundColor Yellow
        Write-Host "The settings below are NOT applied to it. Re-run without -SkipExistingCheck to restart with them." -ForegroundColor Yellow
    }
    else {
        Write-Host "Nothing listening on port $Port; starting a server (existing Ollama processes left alone)."
    }
}
else {
    Write-Host "Stopping existing Ollama processes (tray app + server + runners) so new config applies..."
    # llama-server.exe outlives the ollama serve that spawned it, and an orphaned
    # runner keeps its model in VRAM, so it has to be killed explicitly.
    Get-Process -Name "ollama app", "ollama", "llama-server" -ErrorAction SilentlyContinue | Stop-Process -Force
    Start-Sleep -Seconds 2

    $waiting = 0
    while ((Test-ServerListening) -and $waiting -lt 30) {
        Start-Sleep -Seconds 1
        $waiting++
    }
}

# The request is held by this thread for as long as the script runs, so no
# background job is needed. A zero return means the call failed.
if ([PowerKeeper]::KeepAwake() -eq 0) {
    Write-Host "Failed to suppress sleep (SetThreadExecutionState returned 0)." -ForegroundColor Red
    exit 1
}
Write-Host "Sleep suppressed while this script runs (display may still turn off)." -ForegroundColor Green

if (-not $server) {
    New-Item -ItemType Directory -Force -Path $LogDir | Out-Null
    $stamp  = Get-Date -Format "yyyyMMdd-HHmmss"
    $outLog = Join-Path $LogDir "ollama-serve-$stamp.out.log"
    $errLog = Join-Path $LogDir "ollama-serve-$stamp.err.log"

    Write-Host "Starting ollama serve with:"
    Write-Host "  Context length : $ContextLength"
    Write-Host "  Num parallel   : $NumParallel"
    Write-Host "  Keep alive     : $KeepAlive"
    Write-Host "  Flash attention: $FlashAttention"
    Write-Host "  KV cache type  : $KVCacheType"
    Write-Host "  Max models     : $MaxLoadedModels"
    Write-Host "  Bind address   : $BindAddress"
    Write-Host "  Port           : $Port"
    Write-Host "  Log (server)   : $errLog"

    # ollama serve writes its log to stderr; stdout is captured too, just in case.
    # Without this the hidden window's output is discarded and crashes leave no trace.
    $server = Start-Process -FilePath $ollamaExe -ArgumentList "serve" -PassThru -WindowStyle Hidden `
        -RedirectStandardOutput $outLog -RedirectStandardError $errLog
}

Write-Host "Press Ctrl+C to stop server and restore sleep behavior."

try {
    while (-not $server.HasExited) {
        Start-Sleep -Seconds 5
        [PowerKeeper]::KeepAwake() | Out-Null
    }
    if ($startedByUs) {
        Write-Host "Ollama exited unexpectedly (code $($server.ExitCode)). See $errLog" -ForegroundColor Red
    }
    else {
        Write-Host "The server we attached to (PID $($server.Id)) has exited." -ForegroundColor Red
    }
}
finally {
    if ($startedByUs -and -not $server.HasExited) {
        Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue
        Write-Host "Server stopped; sleep behavior restored."
    }
    else {
        Write-Host "Left the existing server running; sleep behavior restored."
    }
    [PowerKeeper]::Restore() | Out-Null
}
