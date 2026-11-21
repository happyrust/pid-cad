# Exports the sample SmartPlant backups through the command PIDBACKUPEXPORT,
# exports the same packages with pid_publish_xml, compares the two folders
# byte for byte, and sums it all up.
#
# Usage: export-compare.ps1 [-Plant <dir>] [-Small <dir>] [-SmallZip <zip>] [-SqlServer <dir>]
#                           [-Reference <dir>] [-OtherPlant <name>] [-Tools <dir>]
#                           [-PlantExpect <e>] [-PlantRefExpect <e>] [-SmallExpect <e>]
#                           [-SmallBrokenExpect <e>] [-SqlServerExpect <e>] [-PlantLimit 60]
#                           [-Only plant,plant-sqplant,plant-ref,small,small-zip,small-broken,sqlserver]
#                           [-Out <dir>]
#
# Seven exports, one after another: the plant backup under the default plant
# name, under -OtherPlant, and against the reference folder -Reference; the
# small backup from its folder and from its zip; a copy of the small backup in
# which one .pid is made into no drawing; the SQL Server backup. For each, the
# program opens the backup and exports it through the command (the ignored
# test app::commands::pidbackupexport::tests::
# a_real_backup_is_exported_through_the_command_and_stays_as_it_was, for the
# copy a_real_backup_with_a_broken_drawing_names_it_and_writes_the_others),
# then pid_publish_xml exports the very package the program exported, under
# the same plant name and against the same reference folder. The program goes
# first: a backup opened from its zip is restored by the program, and that
# restore is the package both sides export. export_compare.py, next to this
# script, reads what the steps left: one table of the exports, the zip
# compared with its folder, the two plant names, the comparison with the
# reference folder, the broken drawing, and the verdict.
#
# An expectation is key=N or key>N joined by commas; the keys are files,
# written, failed, pid, templates, unmatched, shells, unwritten, and gate,
# whose value is four numbers: clean, with findings, of another revision,
# without a reference (gate=7/0/4/42). -PlantLimit is how many seconds the
# export of the plant backup under the default plant name may take.
#
# pid_publish_xml.exe is taken from -Tools, by default
# D:\scratch\ocs-sppid\tools\<the first seven digits of the pid-parse commit
# that Cargo.toml pins>, where build-pid-tools.ps1 leaves it.
#
# The samples are not in the repository. The defaults are where they lie on the
# machine the plan was written on (docs/plans/2026-11-17-sppid-backup-publish/
# 01-shared-conventions.md, section 2); the default expectations are what that
# plan asks for (its todo.md, EXP-3).
#
# -Out receives everything: export-<name> (the restore, the program's export in
# export, report.json, cargo-test.log, what pid_publish_xml wrote in cli and
# printed in cli.stdout.txt and cli.stderr.txt, and for the copy the copy in
# package), runs.json and summary.md. It must lie outside the repository; the
# default is D:\scratch\ocs-sppid\<date>-export-compare. Those folders of an
# earlier run in -Out are removed first. -Only runs some of the seven; the
# summary then says it is only a part.
#
# cargo runs in the repository this script lies in, offline and with the lock
# file, whatever the current folder is. The table is Chinese Markdown, ready
# for readings.md; summary.md holds the same text as UTF-8.
#
# Exit code: 0 when everything passes; 1 when something does not; 2 when the
# arguments cannot be used, the test program does not build, or the summary
# could not be made.
#
# ASCII only: Windows PowerShell 5.1 misreads UTF-8 without a BOM.
param(
    [string]$Plant = 'D:\work\plant-code\cad\pid-test-data',
    [string]$Small = 'D:\work\plant-code\cad\pid-parse\test-file\backup-test\DWG-0202GP06-01_p',
    [string]$SmallZip = '',
    [string]$SqlServer = 'D:\work\plant-code\cad\pid-parse\test-file\backup-test\TEST02_p',
    [string]$Reference = '',
    [string]$OtherPlant = 'SQPlant',
    [string]$Tools = '',
    [string[]]$PlantExpect = @('files=160,written=53,failed=0,pid=53,templates=0,unmatched=0,shells=2'),
    [string[]]$PlantRefExpect = @('files=160,written=53,failed=0,pid=53,templates=0,unmatched=0,shells=2,gate=7/0/4/42'),
    [string[]]$SmallExpect = @('files=22,written=7,failed=0'),
    [string[]]$SmallBrokenExpect = @('files=19,written=6,unwritten=1,failed>0'),
    [string[]]$SqlServerExpect = @('written=1,pid=12,templates=11'),
    [int]$PlantLimit = 60,
    [string[]]$Only = @(),
    [string]$Out = ''
)

$exportTest = 'app::commands::pidbackupexport::tests::a_real_backup_is_exported_through_the_command_and_stays_as_it_was'
$brokenTest = 'app::commands::pidbackupexport::tests::a_real_backup_with_a_broken_drawing_names_it_and_writes_the_others'
$defaultPlant = 'P01'

function Stop-Usage([string]$message) {
    [Console]::Error.WriteLine("export-compare: $message")
    exit 2
}

function Get-Expectation([string]$name, [string[]]$value) {
    # files=22,written=7 typed in PowerShell arrives as two values; from -File as one.
    $text = ($value -join ',') -replace '\s', ''
    $part = '[a-z]+(=\d+(/\d+){3}|[=>]\d+)'
    if ($text -notmatch "^$part(,$part)*$") {
        Stop-Usage "-$name is '$text'; it takes key=N or key>N joined by commas, as in files=22,written=7."
    }
    return $text
}

function Get-FullPath([string]$path) {
    # Against the current location of PowerShell, whether or not the path exists.
    $full = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($path)
    if ($full.Length -gt 3) { $full = $full.TrimEnd('\', '/') }
    return $full
}

function Remove-Left([string]$path) {
    # What an earlier run left here must not pass for this run.
    if (-not (Test-Path -LiteralPath $path)) { return }
    try {
        Remove-Item -LiteralPath $path -Recurse -Force -ErrorAction Stop
    } catch {
        Stop-Usage "$path is left from an earlier run and cannot be removed: $($_.Exception.Message)"
    }
}

function Stop-Unbuilt([int]$code, [string]$log) {
    # cargo names the test program before it runs it. Without that line the
    # program did not build, and no later step fares better.
    $ran = Select-String -LiteralPath $log -Pattern '^\s+Running ' -Quiet -ErrorAction SilentlyContinue
    if ($code -ne 0 -and -not $ran) {
        Get-Content -LiteralPath $log -Tail 20 -ErrorAction SilentlyContinue |
            ForEach-Object { [Console]::Error.WriteLine($_) }
        Stop-Usage "the test program does not build; see $log"
    }
}

foreach ($tool in 'cargo', 'python') {
    if (-not (Get-Command $tool -CommandType Application -ErrorAction SilentlyContinue)) {
        Stop-Usage "$tool is not on PATH."
    }
}
$expectPlant = Get-Expectation 'PlantExpect' $PlantExpect
$expectPlantRef = Get-Expectation 'PlantRefExpect' $PlantRefExpect
$expectSmall = Get-Expectation 'SmallExpect' $SmallExpect
$expectSmallBroken = Get-Expectation 'SmallBrokenExpect' $SmallBrokenExpect
$expectSqlServer = Get-Expectation 'SqlServerExpect' $SqlServerExpect
if ($PlantLimit -le 0) { Stop-Usage "-PlantLimit is $PlantLimit; it takes a number of seconds." }
if ($OtherPlant -notmatch '^[A-Za-z0-9_]+$' -or $OtherPlant -eq $defaultPlant) {
    Stop-Usage "-OtherPlant is '$OtherPlant'; it takes a plant name of letters and digits other than $defaultPlant."
}
$Plant = Get-FullPath $Plant
$Small = Get-FullPath $Small
if (-not $SmallZip) { $SmallZip = $Small + '.zip' }
# The folder is named in Chinese: yan shou shu ju, the data to accept against.
if (-not $Reference) { $Reference = 'D:\work\cad\pid-test-data\' + (-join [char[]](0x9A8C, 0x6536, 0x6570, 0x636E)) }
$Reference = Get-FullPath $Reference
$sources = @(
    @{ Name = 'plant'; Package = 'plant'; Form = 'folder'; Source = $Plant; Plant = $defaultPlant; Reference = $null;
        Broken = $false; Expect = $expectPlant; Limit = $PlantLimit },
    @{ Name = 'plant-sqplant'; Package = 'plant'; Form = 'folder'; Source = $Plant; Plant = $OtherPlant; Reference = $null;
        Broken = $false; Expect = $expectPlant; Limit = $null },
    @{ Name = 'plant-ref'; Package = 'plant'; Form = 'folder'; Source = $Plant; Plant = $defaultPlant; Reference = $Reference;
        Broken = $false; Expect = $expectPlantRef; Limit = $null },
    @{ Name = 'small'; Package = 'small'; Form = 'folder'; Source = $Small; Plant = $defaultPlant; Reference = $null;
        Broken = $false; Expect = $expectSmall; Limit = $null },
    @{ Name = 'small-zip'; Package = 'small'; Form = 'zip'; Source = (Get-FullPath $SmallZip); Plant = $defaultPlant;
        Reference = $null; Broken = $false; Expect = $expectSmall; Limit = $null },
    @{ Name = 'small-broken'; Package = 'small'; Form = 'folder'; Source = $Small; Plant = $defaultPlant; Reference = $null;
        Broken = $true; Expect = $expectSmallBroken; Limit = $null },
    @{ Name = 'sqlserver'; Package = 'sqlserver'; Form = 'folder'; Source = (Get-FullPath $SqlServer); Plant = $defaultPlant;
        Reference = $null; Broken = $false; Expect = $expectSqlServer; Limit = $null }
)
$known = @($sources | ForEach-Object { $_.Name })
$asked = @($Only | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim().ToLowerInvariant() } | Where-Object { $_ })
foreach ($name in $asked) {
    if ($known -notcontains $name) {
        Stop-Usage "-Only takes $($known -join ', '); '$name' is none of them."
    }
}
$picked = @($sources | Where-Object { $asked.Count -eq 0 -or $asked -contains $_.Name })

$repo = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..')).Path.TrimEnd('\')
if (-not $Tools) {
    $pinned = Select-String -LiteralPath (Join-Path $repo 'Cargo.toml') -Pattern '^pid-parse\s*=.*\brev\s*=\s*"([0-9a-f]{7})' |
        Select-Object -First 1
    if (-not $pinned) {
        Stop-Usage 'Cargo.toml pins no pid-parse commit; name the folder of pid_publish_xml.exe with -Tools.'
    }
    $Tools = 'D:\scratch\ocs-sppid\tools\' + $pinned.Matches[0].Groups[1].Value
}
$Tools = Get-FullPath $Tools
$publish = Join-Path $Tools 'pid_publish_xml.exe'
if (-not (Test-Path -LiteralPath $publish -PathType Leaf)) {
    Stop-Usage "no pid_publish_xml.exe in $Tools; scripts\sppid\build-pid-tools.ps1 builds it."
}

if (-not $Out) { $Out = 'D:\scratch\ocs-sppid\{0}-export-compare' -f (Get-Date -Format 'yyyy-MM-dd') }
$Out = Get-FullPath $Out
if (($Out + '\').StartsWith($repo + '\', [StringComparison]::OrdinalIgnoreCase)) {
    Stop-Usage "-Out is $Out, inside the repository; the exports and reports stay outside it."
}
try {
    New-Item -ItemType Directory -Force -Path $Out -ErrorAction Stop | Out-Null
} catch {
    Stop-Usage "-Out $Out cannot be made: $($_.Exception.Message)"
}

# The tests read these; whatever the caller had set comes back at the end.
$variables = 'OCS_PID_BACKUP_EXPORT_REAL', 'OCS_PID_BACKUP_EXPORT_REAL_PLANT', 'OCS_PID_BACKUP_EXPORT_REAL_REF',
    'OCS_PID_BACKUP_EXPORT_REAL_OUT', 'OCS_PID_BACKUP_EXPORT_BROKEN', 'OCS_PID_BACKUP_EXPORT_BROKEN_OUT'
$before = @{}
foreach ($name in $variables) { $before[$name] = [Environment]::GetEnvironmentVariable($name) }
$runs = @()
$build = ''
$count = $picked.Count
Write-Host "$count export(s), one after another; everything goes to $Out"
Push-Location -LiteralPath $repo
try {
    $at = 0
    foreach ($sample in $picked) {
        $at++
        $folder = Join-Path $Out ('export-' + $sample.Name)
        Remove-Left $folder
        New-Item -ItemType Directory -Force -Path $folder | Out-Null
        $tool = [ordered]@{
            folder = (Join-Path $folder 'cli')
            stdout = (Join-Path $folder 'cli.stdout.txt')
            stderr = (Join-Path $folder 'cli.stderr.txt')
            exit_code = $null
            seconds = $null
        }
        $run = [ordered]@{
            name = $sample.Name
            package = $sample.Package
            form = $sample.Form
            source = $sample.Source
            plant = $sample.Plant
            reference = $sample.Reference
            broken = $sample.Broken
            expect = $sample.Expect
            folder = $folder
            exit_code = $null
            seconds = $null
            limit_seconds = $sample.Limit
            note = ''
            tool = $tool
        }
        $label = '[{0}/{1}] {2}' -f $at, $count, $sample.Name
        if (-not (Test-Path -LiteralPath $sample.Source)) {
            $run.note = 'the source does not exist'
            Write-Host "${label}: not run, $($sample.Source) does not exist"
        } elseif ($sample.Reference -and -not (Test-Path -LiteralPath $sample.Reference -PathType Container)) {
            $run.note = 'the reference folder does not exist'
            Write-Host "${label}: not run, $($sample.Reference) does not exist"
        } else {
            foreach ($name in $variables) { Remove-Item "Env:$name" -ErrorAction SilentlyContinue }
            if ($sample.Broken) {
                $env:OCS_PID_BACKUP_EXPORT_BROKEN = $sample.Source
                $env:OCS_PID_BACKUP_EXPORT_BROKEN_OUT = $folder
                $test = $brokenTest
            } else {
                $env:OCS_PID_BACKUP_EXPORT_REAL = $sample.Source
                $env:OCS_PID_BACKUP_EXPORT_REAL_OUT = $folder
                # The default plant name is the one the command takes when none is typed.
                if ($sample.Plant -ne $defaultPlant) { $env:OCS_PID_BACKUP_EXPORT_REAL_PLANT = $sample.Plant }
                if ($sample.Reference) { $env:OCS_PID_BACKUP_EXPORT_REAL_REF = $sample.Reference }
                $test = $exportTest
            }
            $log = Join-Path $folder 'cargo-test.log'
            $watch = [Diagnostics.Stopwatch]::StartNew()
            cmd /c "cargo test --offline --locked --lib $test -- --ignored --nocapture --exact > `"$log`" 2>&1"
            $run.exit_code = $LASTEXITCODE
            $run.seconds = [math]::Round($watch.Elapsed.TotalSeconds, 1)
            Write-Host ('{0}: export exit={1} seconds={2}' -f $label, $run.exit_code, $run.seconds)
            Stop-Unbuilt $run.exit_code $log
            if (-not $build) {
                $finished = Select-String -LiteralPath $log -Pattern '^\s*Finished ' | Select-Object -First 1
                if ($finished) { $build = $finished.Line.Trim() }
            }
            # pid_publish_xml exports the package the program exported: for a
            # zip that is the program's restore, for the copy the copy.
            $package = ''
            $report = Join-Path $folder 'report.json'
            if (Test-Path -LiteralPath $report) {
                try {
                    $told = Get-Content -LiteralPath $report -Raw -Encoding UTF8 | ConvertFrom-Json
                    $package = [string]$told.package
                } catch {
                    $package = ''
                }
            }
            if ($package -and (Test-Path -LiteralPath $package -PathType Container)) {
                $diff = ''
                if ($sample.Reference) { $diff = " --diff-against `"$($sample.Reference)`"" }
                $watch = [Diagnostics.Stopwatch]::StartNew()
                cmd /c "`"$publish`" `"$package`" --out-dir `"$($tool.folder)`" --plant $($sample.Plant)$diff > `"$($tool.stdout)`" 2> `"$($tool.stderr)`""
                $tool.exit_code = $LASTEXITCODE
                $tool.seconds = [math]::Round($watch.Elapsed.TotalSeconds, 1)
                Write-Host ('{0}: pid_publish_xml exit={1} seconds={2}' -f $label, $tool.exit_code, $tool.seconds)
            } else {
                Write-Host "${label}: pid_publish_xml not run, the export names no package to publish"
            }
        }
        $runs += $run
    }
} finally {
    Pop-Location
    foreach ($name in $variables) { [Environment]::SetEnvironmentVariable($name, $before[$name]) }
}

$manifest = [ordered]@{
    partial = ($count -lt $known.Count)
    build = $build
    tools = $Tools
    runs = $runs
}
$runsFile = Join-Path $Out 'runs.json'
[IO.File]::WriteAllText($runsFile, ($manifest | ConvertTo-Json -Depth 6), (New-Object Text.UTF8Encoding($false)))

$summary = Join-Path $Out 'summary.md'
if (Test-Path -LiteralPath $summary) { Remove-Item -LiteralPath $summary -Force }
# The summary is Chinese: python writes UTF-8, and the console is told to expect it.
$encoding = $env:PYTHONIOENCODING
$console = $null
try {
    $console = [Console]::OutputEncoding
    [Console]::OutputEncoding = New-Object Text.UTF8Encoding($false)
} catch {
    $console = $null
}
$env:PYTHONIOENCODING = 'utf-8'
try {
    python (Join-Path $PSScriptRoot 'export_compare.py') $runsFile --out $summary
    $verdict = $LASTEXITCODE
} finally {
    $env:PYTHONIOENCODING = $encoding
    if ($console) { try { [Console]::OutputEncoding = $console } catch { } }
}
if (-not (Test-Path -LiteralPath $summary)) {
    Stop-Usage "the summary could not be made from $runsFile"
}
Write-Host "summary: $summary"
exit $verdict
