# Controlled SmartPlant P&ID verification runner scaffold (LX-2 / LX-8 / #144).
# Compatible with Windows PowerShell 5.1; strictly 7-bit ASCII only.
#
# Contract subcommands (invoked by scripts/sppid/precheck_and_receipt.py):
#   open    --in <bundle_dir> --out <out_dir> [--timeout <sec>] [--dry-run]
#   publish --in <bundle_dir> --out <out_dir> [--timeout <sec>] [--dry-run]
#   restore --snapshot <name> --out <out_dir> [--timeout <sec>] [--dry-run]

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Write-Utf8NoBom([string]$FilePath, [string]$Text) {
    $parent = Split-Path -Parent $FilePath
    if ($parent -and -not (Test-Path -LiteralPath $parent)) {
        New-Item -ItemType Directory -Force -Path $parent | Out-Null
    }
    $enc = New-Object System.Text.UTF8Encoding($false)
    [System.IO.File]::WriteAllText($FilePath, $Text, $enc)
}

$dryRun = $false
$subcmd = ''
$inDir = ''
$outDir = ''
$snapshotName = 'baseline'
$timeoutSec = 180

$i = 0
while ($i -lt $args.Count) {
    $tok = [string]$args[$i]
    switch ($tok.ToLowerInvariant()) {
        '--dry-run' {
            $dryRun = $true
            $i += 1
        }
        '--in' {
            if ($i + 1 -ge $args.Count) { throw 'missing value for --in' }
            $inDir = [string]$args[$i + 1]
            $i += 2
        }
        '--out' {
            if ($i + 1 -ge $args.Count) { throw 'missing value for --out' }
            $outDir = [string]$args[$i + 1]
            $i += 2
        }
        '--snapshot' {
            if ($i + 1 -ge $args.Count) { throw 'missing value for --snapshot' }
            $snapshotName = [string]$args[$i + 1]
            $i += 2
        }
        '--timeout' {
            if ($i + 1 -ge $args.Count) { throw 'missing value for --timeout' }
            $timeoutSec = [int]$args[$i + 1]
            $i += 2
        }
        default {
            if (-not $subcmd -and $tok -in @('open', 'publish', 'restore')) {
                $subcmd = $tok.ToLowerInvariant()
                $i += 1
            } else {
                throw "unexpected argument: $tok"
            }
        }
    }
}

if (-not $subcmd) {
    [Console]::Error.WriteLine('usage: sppid_runner_template.ps1 [--dry-run] <open|publish|restore> --out <dir> [--in <dir>] [--snapshot <name>] [--timeout <sec>]')
    exit 2
}
if (-not $outDir) {
    [Console]::Error.WriteLine('error: --out <dir> is required')
    exit 2
}
New-Item -ItemType Directory -Force -Path $outDir | Out-Null

function Invoke-SppidNativeOpen([string]$PidFile, [int]$TimeoutSeconds) {
    # Hook point for controlled VM execution ($env:OCS_SPPID_OPEN_CMD).
    $cmdTpl = $env:OCS_SPPID_OPEN_CMD
    if (-not $cmdTpl) {
        return @{ opened = $false; message = 'OCS_SPPID_OPEN_CMD not configured in controlled guest' }
    }
    $proc = Start-Process -FilePath $cmdTpl -ArgumentList @($PidFile) -PassThru -NoNewWindow
    if (-not $proc.WaitForExit($TimeoutSeconds * 1000)) {
        Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
        return @{ opened = $false; message = "timeout after ${TimeoutSeconds}s" }
    }
    return @{ opened = ($proc.ExitCode -eq 0); message = "exit=$($proc.ExitCode)" }
}

if ($subcmd -eq 'open') {
    if (-not $inDir -or -not (Test-Path -LiteralPath $inDir)) {
        [Console]::Error.WriteLine("error: --in directory does not exist: $inDir")
        exit 2
    }
    $pids = @(Get-ChildItem -LiteralPath $inDir -Filter '*.pid' -File | Sort-Object Name)
    $rows = @()
    $allOk = $true
    foreach ($pf in $pids) {
        if ($dryRun) {
            $rows += [ordered]@{
                file = $pf.Name
                opened = $true
                message = 'dry-run simulated open ok'
            }
        } else {
            $r = Invoke-SppidNativeOpen -PidFile $pf.FullName -TimeoutSeconds $timeoutSec
            if (-not $r.opened) { $allOk = $false }
            $rows += [ordered]@{
                file = $pf.Name
                opened = [bool]$r.opened
                message = [string]$r.message
            }
        }
    }
    $payload = [ordered]@{
        mode = if ($dryRun) { 'dry_run' } else { 'controlled_vm' }
        timeout_sec = $timeoutSec
        files = $rows
    }
    $json = ($payload | ConvertTo-Json -Depth 6) + "`n"
    Write-Utf8NoBom -FilePath (Join-Path $outDir 'open-result.json') -Text $json
    if ($allOk) { exit 0 } else { exit 1 }
}

if ($subcmd -eq 'publish') {
    if (-not $inDir -or -not (Test-Path -LiteralPath $inDir)) {
        [Console]::Error.WriteLine("error: --in directory does not exist: $inDir")
        exit 2
    }
    $pids = @(Get-ChildItem -LiteralPath $inDir -Filter '*.pid' -File | Sort-Object Name)
    $rows = @()
    $allOk = $true
    foreach ($pf in $pids) {
        $stem = [System.IO.Path]::GetFileNameWithoutExtension($pf.Name)
        if ($dryRun) {
            foreach ($sfx in @('_Data.xml', '_Meta.xml')) {
                $srcXml = Join-Path $inDir ($stem + $sfx)
                if (Test-Path -LiteralPath $srcXml) {
                    Copy-Item -LiteralPath $srcXml -Destination (Join-Path $outDir ($stem + $sfx)) -Force
                }
            }
            $rows += [ordered]@{
                file = $pf.Name
                published = $true
                message = 'dry-run simulated publish ok'
            }
        } else {
            $cmdTpl = $env:OCS_SPPID_PUBLISH_CMD
            if (-not $cmdTpl) {
                $allOk = $false
                $rows += [ordered]@{
                    file = $pf.Name
                    published = $false
                    message = 'OCS_SPPID_PUBLISH_CMD not configured in controlled guest'
                }
            } else {
                $proc = Start-Process -FilePath $cmdTpl -ArgumentList @($pf.FullName, $outDir) -PassThru -NoNewWindow
                if (-not $proc.WaitForExit($timeoutSec * 1000)) {
                    Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
                    $allOk = $false
                    $rows += [ordered]@{ file = $pf.Name; published = $false; message = "timeout after ${timeoutSec}s" }
                } else {
                    $ok = ($proc.ExitCode -eq 0)
                    if (-not $ok) { $allOk = $false }
                    $rows += [ordered]@{ file = $pf.Name; published = $ok; message = "exit=$($proc.ExitCode)" }
                }
            }
        }
    }
    $payload = [ordered]@{
        mode = if ($dryRun) { 'dry_run' } else { 'controlled_vm' }
        timeout_sec = $timeoutSec
        files = $rows
    }
    $json = ($payload | ConvertTo-Json -Depth 6) + "`n"
    Write-Utf8NoBom -FilePath (Join-Path $outDir 'publish-result.json') -Text $json
    if ($allOk) { exit 0 } else { exit 1 }
}

if ($subcmd -eq 'restore') {
    $restored = $false
    $msg = ''
    if ($dryRun) {
        $restored = $true
        $msg = "dry-run simulated snapshot restore: $snapshotName"
    } else {
        $cmdTpl = $env:OCS_SPPID_RESTORE_CMD
        if (-not $cmdTpl) {
            $restored = $false
            $msg = 'OCS_SPPID_RESTORE_CMD not configured on host'
        } else {
            $proc = Start-Process -FilePath $cmdTpl -ArgumentList @($snapshotName) -PassThru -NoNewWindow -Wait
            $restored = ($proc.ExitCode -eq 0)
            $msg = "exit=$($proc.ExitCode)"
        }
    }
    $payload = [ordered]@{
        mode = if ($dryRun) { 'dry_run' } else { 'controlled_vm' }
        snapshot = $snapshotName
        restored = $restored
        message = $msg
    }
    $json = ($payload | ConvertTo-Json -Depth 6) + "`n"
    Write-Utf8NoBom -FilePath (Join-Path $outDir 'restore-result.json') -Text $json
    if ($restored) { exit 0 } else { exit 1 }
}
