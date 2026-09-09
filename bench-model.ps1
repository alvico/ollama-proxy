<#
    Measures a model's eval throughput on both tuned backends so a routing
    decision in config.json can be made from numbers rather than a guess.

    Hits the backends directly (11435/11436), bypassing the proxy, because the
    proxy drains the other backend on every switch and that is exactly the
    behaviour being measured around. Each backend is explicitly unloaded before
    and after its runs so neither holds VRAM while the other is timed.

    Matches the methodology already recorded in config.json: ctx 16384,
    num_parallel 1 (set at server start), temperature 0, eval tok/s.

    Usage:
      .\bench-model.ps1 -Model qwen3.6:35b-a3b-coding
      .\bench-model.ps1 -Model qwen3.6:35b-a3b-coding -NumCtx 32768 -Runs 5
#>
param(
    [Parameter(Mandatory = $true)][string]$Model,
    [int]$Q8Port    = 11435,
    [int]$F16Port   = 11436,
    [int]$NumCtx    = 16384,
    [int]$NumPredict = 256,
    [int]$Runs      = 3,
    [string]$Prompt = "Write a Rust function that merges two sorted slices of i32 into a new Vec, without using sort. Explain the invariant that makes it correct."
)

$ErrorActionPreference = "Stop"

function Unload-Model {
    param([int]$Port, [string]$Model)
    $body = @{ model = $Model; keep_alive = 0 } | ConvertTo-Json -Compress
    try {
        Invoke-RestMethod -Method Post -Uri "http://127.0.0.1:$Port/api/generate" `
            -Body $body -ContentType "application/json" -TimeoutSec 120 | Out-Null
    } catch {
        # A model that was never loaded unloads fine; nothing to report.
    }
}

function Test-Backend {
    param([int]$Port)
    try {
        Invoke-RestMethod -Method Get -Uri "http://127.0.0.1:$Port/api/tags" -TimeoutSec 10 | Out-Null
        return $true
    } catch {
        return $false
    }
}

function Measure-Backend {
    param([string]$Name, [int]$Port, [string]$Model)

    Write-Host ""
    Write-Host "=== $Name (127.0.0.1:$Port) ===" -ForegroundColor Cyan

    if (-not (Test-Backend -Port $Port)) {
        Write-Host "  backend not responding - is start-stack.ps1 running?" -ForegroundColor Red
        return $null
    }

    # Free the other backend so this one is measured with the card to itself.
    $other = if ($Port -eq $Q8Port) { $F16Port } else { $Q8Port }
    Unload-Model -Port $other -Model $Model

    $body = @{
        model      = $Model
        prompt     = $Prompt
        stream     = $false
        think      = $false
        keep_alive = "5m"
        options    = @{
            num_ctx     = $NumCtx
            num_predict = $NumPredict
            temperature = 0
        }
    } | ConvertTo-Json -Depth 5 -Compress

    # Warmup: loads weights and pays the offload cost once so it is not timed.
    Write-Host "  warmup (model load can take minutes when partially offloaded)..."
    try {
        $warm = Invoke-RestMethod -Method Post -Uri "http://127.0.0.1:$Port/api/generate" `
            -Body $body -ContentType "application/json" -TimeoutSec 3600
    } catch {
        Write-Host "  FAILED: $($_.Exception.Message)" -ForegroundColor Red
        Write-Host "  (on the q8 backend this is often the KV block-size constraint:" -ForegroundColor Yellow
        Write-Host "   q8_0 quantises in blocks of 32, which must divide n_embd_head_v)" -ForegroundColor Yellow
        return $null
    }
    Write-Host ("  loaded in {0:N1}s" -f ($warm.load_duration / 1e9))

    $rates = @()
    for ($i = 1; $i -le $Runs; $i++) {
        $r = Invoke-RestMethod -Method Post -Uri "http://127.0.0.1:$Port/api/generate" `
            -Body $body -ContentType "application/json" -TimeoutSec 3600

        $evalRate   = $r.eval_count / ($r.eval_duration / 1e9)
        $promptRate = if ($r.prompt_eval_duration -gt 0) {
            $r.prompt_eval_count / ($r.prompt_eval_duration / 1e9)
        } else { 0 }
        $rates += $evalRate

        Write-Host ("  run {0}: {1,6:N1} eval tok/s   ({2} tokens, prompt {3,6:N1} tok/s)" -f `
            $i, $evalRate, $r.eval_count, $promptRate)
    }

    Unload-Model -Port $Port -Model $Model

    $stats = $rates | Measure-Object -Average -Maximum -Minimum
    [PSCustomObject]@{
        Backend = $Name
        Mean    = $stats.Average
        Min     = $stats.Minimum
        Max     = $stats.Maximum
    }
}

Write-Host "Benchmarking $Model at ctx $NumCtx, $Runs runs per backend." -ForegroundColor Green

$results = @(
    Measure-Backend -Name "f16" -Port $F16Port -Model $Model
    Measure-Backend -Name "q8"  -Port $Q8Port  -Model $Model
) | Where-Object { $_ -ne $null }

Write-Host ""
Write-Host "=== Summary ===" -ForegroundColor Green
$results | Format-Table Backend, @{L = "Mean tok/s"; E = { "{0:N1}" -f $_.Mean } },
                                 @{L = "Min"; E = { "{0:N1}" -f $_.Min } },
                                 @{L = "Max"; E = { "{0:N1}" -f $_.Max } } -AutoSize

$f16 = $results | Where-Object Backend -eq "f16"
$q8  = $results | Where-Object Backend -eq "q8"

if ($f16 -and $q8) {
    $delta = ($q8.Mean - $f16.Mean) / $f16.Mean * 100
    Write-Host ("q8 vs f16: {0:+0.0;-0.0}%" -f $delta)
    Write-Host ""
    if ($delta -gt 3) {
        Write-Host "-> Route it to q8. Add to config.json routes:" -ForegroundColor Yellow
        Write-Host "     `"$Model`": `"q8`"" -ForegroundColor Yellow
        Write-Host "   (use the full tag as the prefix so sibling tags keep their own routing)"
    } else {
        Write-Host "-> Leave it on the f16 default; q8 buys nothing here." -ForegroundColor Yellow
        Write-Host "   Record the measurement in the _routes comment so the decision is not re-litigated."
    }
} elseif ($f16) {
    Write-Host "q8 backend could not serve this model; f16 default is the only option." -ForegroundColor Yellow
}
