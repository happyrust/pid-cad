# Runs the end-to-end scenarios in eight groups, one after another, and writes
# one summary: the harness's own scenarios (t00_smoke, t02_harness_rules,
# t03_vanish_retry, t04_entry_rerun), the eleven conflict and cross-area
# scenarios (run_cross.ps1), the eight desktop-client scenarios (run_all.ps1), the later
# cross-area scenarios (every y<digit>*.py in this directory), then the
# server-process scenarios (every s<digit>*.py: the collaboration server and
# WS sessions only, no GUI), then the picture scenarios (every g<digit>*.py:
# one GUI each, judged by the screenshots it takes of itself), then the timing
# scenarios (every p<digit>*.py: the contract's time limits, measured), then
# the offline scenarios (every o<digit>*.py: steps made apart from the room,
# across restarts, and copies the room will not take as aligned).
# Whatever was picked, r01_entry_rules.py runs last:
# it checks the rules of the run as a whole (nothing of the run in the
# repository, the port of every GUI start, the dependency tree).
#
#   powershell -File run19.ps1 [-Label <name>] [-OnlyHarness] [-Only19] [-OnlyNew] [-OnlyServer] [-OnlyPictures]
#                              [-OnlyTiming] [-OnlyOffline] [-EvidenceDir <dir>] [-BinDir <dir>] [-PortShift <n>]
#
# -Label     names this run: the summary is summary-<Label>-<time>.txt in the
#            evidence directory.
# -OnlyHarness only the first group; -Only19 only the second and the third;
#            -OnlyNew only the fourth; -OnlyServer only the fifth;
#            -OnlyPictures only the sixth; -OnlyTiming only the seventh;
#            -OnlyOffline only the eighth. Given together, they add up.
# -EvidenceDir, -BinDir and -PortShift are explained in e2e_env.ps1; each can
# be given by its environment variable instead.
#
# Drawings the scenarios read but do not make are made first when missing: the
# M3 baseline drawing (by the ignored test write_the_m3_baseline_drawing, which
# needs cargo) and the two seed drawings (x00_seed.py, t01_seed.py). The
# harness's own, the server-process, the picture and the timing scenarios read
# none of them, so -OnlyHarness, -OnlyServer, -OnlyPictures and -OnlyTiming
# make none. (Of the timing scenarios, p03_tag_rule_budget.py needs cargo as
# well: it runs one ignored test of the library.) The offline scenarios read
# the seed drawing x00_seed.py makes, and only that one, so -OnlyOffline makes
# only that one.
# A scenario that loses an application instance is run once more and both
# results are kept (Invoke-E2eScenario in e2e_env.ps1); the summary shows the
# first attempt on a line of its own and does not count it. A scenario is
# clean when no check failed and its process ended with exit code 0.
# No process this run did not start is touched; the summary only records how
# many other instances of the two programs were running, to explain a failure by.
# The only git command is the read-only `git status` of r01_entry_rules.py.
param([string]$Label = 'run', [switch]$OnlyHarness, [switch]$Only19, [switch]$OnlyNew, [switch]$OnlyServer,
      [switch]$OnlyPictures, [switch]$OnlyTiming, [switch]$OnlyOffline, [string]$EvidenceDir = '', [string]$BinDir = '',
      [string]$PortShift = '')
$picked = $OnlyHarness -or $Only19 -or $OnlyNew -or $OnlyServer -or $OnlyPictures -or $OnlyTiming -or $OnlyOffline
$runHarness = (-not $picked) -or $OnlyHarness
$run19 = (-not $picked) -or $Only19
$runNew = (-not $picked) -or $OnlyNew
$runServer = (-not $picked) -or $OnlyServer
$runPictures = (-not $picked) -or $OnlyPictures
$runTiming = (-not $picked) -or $OnlyTiming
$runOffline = (-not $picked) -or $OnlyOffline
$ErrorActionPreference = 'Continue'
try { [Console]::OutputEncoding = [Text.Encoding]::UTF8 } catch { }
$env:PYTHONIOENCODING = 'utf-8'
. (Join-Path $PSScriptRoot 'e2e_env.ps1')
$e2e = Resolve-E2eEnv -Here $PSScriptRoot -EvidenceDir $EvidenceDir -BinDir $BinDir -PortShift $PortShift
$root = $e2e.Evidence
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$out = Join-Path $root "summary-$Label-$stamp.txt"
$cross = Join-Path $e2e.Scripts 'run_cross.ps1'
$client = Join-Path $e2e.Scripts 'run_all.ps1'

function Say([string]$text) { $text | Out-File -Encoding utf8 -Append $out; Write-Host $text }

$version = (& (Join-Path $e2e.Bin 'OpenCADStudio.exe') --version 2>&1 | Select-Object -First 2) -join ' / '
Say "### $(Get-Date -Format s) label=$Label bin=$($e2e.Bin) port-shift=$($e2e.PortShift) :: $version"
Say "other instances running at start: $(@(Get-E2eProcesses $e2e -Other).Count)"

$m7work = Join-Path $root 'm7\work'; $m6work = Join-Path $root 'm6\work'
$m7logs = Join-Path $root 'm7\logs'; $m6logs = Join-Path $root 'm6\logs'
New-Item -ItemType Directory -Force -Path $m7work, $m6work, $m7logs, $m6logs | Out-Null

# Before anything runs: when the run began and what the repository looked
# like. r01_entry_rules.py compares with it at the end.
Push-Location $e2e.Scripts
python r01_entry_rules.py --begin *> (Join-Path $m7logs 'r01_begin.console.txt'); Say "run rules: the state before the run is recorded, exit=$LASTEXITCODE"
Pop-Location

# Inputs the scenarios read but do not make.
$m3 = Join-Path $m7work 'PID-1101-m3.dwg'
$drawings = $run19 -or $runNew
if ($drawings -and -not (Test-Path $m3)) {
    $env:OCS_M3_BASELINE_OUT = $m3
    Push-Location $e2e.Repo
    cmd /c "cargo test --offline --locked --lib -- --ignored write_the_m3_baseline_drawing > `"$m7logs\m3_baseline.console.txt`" 2>&1"
    Say "m3 baseline: cargo exit=$LASTEXITCODE written=$(Test-Path $m3)"
    Pop-Location
    Remove-Item Env:OCS_M3_BASELINE_OUT
}
Push-Location $e2e.Scripts
if (($drawings -or $runOffline) -and -not (Test-Path (Join-Path $m7work 'PID-1101-seed.dwg'))) {
    Invoke-E2eScenario -E2e $e2e -Suite 'm7' -Name 'x00_seed' -Summary $out -Descriptors
}
if ($drawings -and -not (Test-Path (Join-Path $m6work 'PID-1101-seed.dwg'))) {
    Invoke-E2eScenario -E2e $e2e -Suite 'm6' -Name 't01_seed' -Summary $out
}
Pop-Location

$rows = @()
function Read-Row([string]$suite, [string]$name, [string]$log, [int]$code, [bool]$first) {
    $tail = if (Test-Path $log) { (Get-Content $log -Encoding UTF8 | Select-String -Pattern '=== .*: (\d+) PASS / (\d+) FAIL ===' | Select-Object -Last 1) } else { $null }
    $pass = 0; $fail = -1
    if ($tail) { $pass = [int]$tail.Matches[0].Groups[1].Value; $fail = [int]$tail.Matches[0].Groups[2].Value }
    [pscustomobject]@{ Suite = $suite; Name = $name; Pass = $pass; Fail = $fail; Exit = $code; First = $first }
}
# One row per scenario, from its console log (the checks) and its runs file
# (the exit code of each attempt). A first attempt that lost an instance gets
# a row of its own, marked First.
function Collect([string]$suite, [string[]]$names) {
    foreach ($name in $names) {
        $logs = Join-Path $root "$suite\logs"
        $runs = Join-Path $logs "$name.runs.txt"
        $codes = @()
        if (Test-Path $runs) {
            $codes = @(Get-Content $runs | Where-Object { $_ -match '^attempt=\d+ exit=-?\d+' } |
                ForEach-Object { [int]([regex]::Match($_, 'exit=(-?\d+)').Groups[1].Value) })
        }
        if ($codes.Count -gt 1) {
            $script:rows += Read-Row $suite "$name.attempt1" (Join-Path $logs "$name.attempt1.console.txt") $codes[0] $true
        }
        $last = -1
        if ($codes.Count -gt 0) { $last = $codes[$codes.Count - 1] }
        $script:rows += Read-Row $suite $name (Join-Path $logs "$name.console.txt") $last $false
    }
}

if ($runHarness) {
    $harness = 't00_smoke', 't02_harness_rules', 't03_vanish_retry', 't04_entry_rerun'
    & powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File $client -Only ($harness -join ',') -New | ForEach-Object { Say "  $_" }
    Collect 'm6' $harness
}
if ($run19) {
    $m7 = 'c01_field_merge', 'c02_object_topology', 'c03_resolution', 'x01_journey', 'x02_claim_exclusive', 'x04_readonly_role',
          'x05_m3_persistence', 'x06_first_visit', 'x07_conflict_closure', 'x10_concurrent_insert', 'x12_after_session'
    $m6 = 't20_session', 't21_claims', 't23_stale', 't24_reconnect', 't25_offline', 't10_broadcast', 't11_same_user', 't22_ttl'
    & powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File $cross | ForEach-Object { Say "  $_" }
    Collect 'm7' $m7
    & powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File $client | ForEach-Object { Say "  $_" }
    Collect 'm6' $m6
}
if ($runNew) {
    $new = @(Get-ChildItem $e2e.Scripts -Filter 'y*.py' | Where-Object { $_.Name -match '^y\d' } |
        Sort-Object Name | ForEach-Object { $_.BaseName })
    if ($new.Count -gt 0) {
        & powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File $cross -Only ($new -join ',') -New | ForEach-Object { Say "  $_" }
        Collect 'm7' $new
    }
}
if ($runServer) {
    $server = @(Get-ChildItem $e2e.Scripts -Filter 's*.py' | Where-Object { $_.Name -match '^s\d' } |
        Sort-Object Name | ForEach-Object { $_.BaseName })
    if ($server.Count -gt 0) {
        & powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File $cross -Only ($server -join ',') -New | ForEach-Object { Say "  $_" }
        Collect 'm7' $server
    }
}
if ($runPictures) {
    $pictures = @(Get-ChildItem $e2e.Scripts -Filter 'g*.py' | Where-Object { $_.Name -match '^g\d' } |
        Sort-Object Name | ForEach-Object { $_.BaseName })
    if ($pictures.Count -gt 0) {
        & powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File $cross -Only ($pictures -join ',') -New | ForEach-Object { Say "  $_" }
        Collect 'm7' $pictures
    }
}
if ($runTiming) {
    $timing = @(Get-ChildItem $e2e.Scripts -Filter 'p*.py' | Where-Object { $_.Name -match '^p\d' } |
        Sort-Object Name | ForEach-Object { $_.BaseName })
    if ($timing.Count -gt 0) {
        & powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File $cross -Only ($timing -join ',') -New | ForEach-Object { Say "  $_" }
        Collect 'm7' $timing
    }
}
if ($runOffline) {
    $offline = @(Get-ChildItem $e2e.Scripts -Filter 'o*.py' | Where-Object { $_.Name -match '^o\d' } |
        Sort-Object Name | ForEach-Object { $_.BaseName })
    if ($offline.Count -gt 0) {
        & powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File $cross -Only ($offline -join ',') -New | ForEach-Object { Say "  $_" }
        Collect 'm7' $offline
    }
}
# The rules of the run as a whole, last.
& powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File $cross -Only 'r01_entry_rules' -New | ForEach-Object { Say "  $_" }
Collect 'm7' @('r01_entry_rules')

Say ''
foreach ($row in $rows) {
    $note = ''
    if ($row.First) { $note = '  (first attempt: lost an instance; kept, not counted)' }
    Say (("{0,-3} {1,-28} pass={2,3} fail={3,2} exit={4}" -f $row.Suite, $row.Name, $row.Pass, $row.Fail, $row.Exit) + $note)
}
$counted = @($rows | Where-Object { -not $_.First })
$harnessRows = @($counted | Where-Object { $_.Name -match '^t0\d' })
$old = @($counted | Where-Object { $_.Name -notmatch '^([ysrgpo]\d|t0\d)' })
$newRows = @($counted | Where-Object { $_.Name -match '^y\d' })
$serverRows = @($counted | Where-Object { $_.Name -match '^s\d' })
$pictureRows = @($counted | Where-Object { $_.Name -match '^g\d' })
$timingRows = @($counted | Where-Object { $_.Name -match '^p\d' })
$offlineRows = @($counted | Where-Object { $_.Name -match '^o\d' })
$ruleRows = @($counted | Where-Object { $_.Name -match '^r\d' })
$sum = { param($set) (($set | Measure-Object -Property Pass -Sum).Sum, (($set | Where-Object { $_.Fail -ne 0 -or $_.Exit -ne 0 }) | Measure-Object).Count) }
if ($harnessRows.Count) { $s = & $sum $harnessRows; Say ("harness scenarios: {0} ({1} checks passed, {2} scenario(s) not clean)" -f $harnessRows.Count, $s[0], $s[1]) }
if ($old.Count) { $s = & $sum $old; Say ("old scenarios: {0} ({1} checks passed, {2} scenario(s) not clean)" -f $old.Count, $s[0], $s[1]) }
if ($newRows.Count) { $s = & $sum $newRows; Say ("new scenarios: {0} ({1} checks passed, {2} scenario(s) not clean)" -f $newRows.Count, $s[0], $s[1]) }
if ($serverRows.Count) { $s = & $sum $serverRows; Say ("server scenarios: {0} ({1} checks passed, {2} scenario(s) not clean)" -f $serverRows.Count, $s[0], $s[1]) }
if ($pictureRows.Count) { $s = & $sum $pictureRows; Say ("picture scenarios: {0} ({1} checks passed, {2} scenario(s) not clean)" -f $pictureRows.Count, $s[0], $s[1]) }
if ($timingRows.Count) { $s = & $sum $timingRows; Say ("timing scenarios: {0} ({1} checks passed, {2} scenario(s) not clean)" -f $timingRows.Count, $s[0], $s[1]) }
if ($offlineRows.Count) { $s = & $sum $offlineRows; Say ("offline scenarios: {0} ({1} checks passed, {2} scenario(s) not clean)" -f $offlineRows.Count, $s[0], $s[1]) }
if ($ruleRows.Count) { $s = & $sum $ruleRows; Say ("run rules: {0} ({1} checks passed, {2} scenario(s) not clean)" -f $ruleRows.Count, $s[0], $s[1]) }
$twice = @($rows | Where-Object { $_.First } | ForEach-Object { $_.Name -replace '\.attempt1$', '' })
if ($twice.Count) { Say "run twice (the first attempt lost an instance): $($twice -join ', ')" }
Say "other instances running at end: $(@(Get-E2eProcesses $e2e -Other).Count)"
Say "### $(Get-Date -Format s) done -> $out"
