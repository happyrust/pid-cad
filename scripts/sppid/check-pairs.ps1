# Checks that the two exports of one SmartPlant backup pair up in the folder
# they were both written to: the publish files PIDBACKUPEXPORT writes and the
# PDFs PIDBACKUPPDF writes.
#
# Usage: check-pairs.ps1 <folder> [-ExpectPairs <n>] [-ExpectFiles <n>] [-Report <file>]
#
# The folder holds _Index.json, which PIDBACKUPEXPORT ends with, and
# _PdfIndex.json, which PIDBACKUPPDF ends with, and the files they name. A
# drawing is one .pid of the package: an entry of the drawings of _Index.json
# by its zip_entry, a page of _PdfIndex.json by its pid, both read with / for \
# and without regard to case. A page whose drawing the backup holds no file of
# (pid null) goes by its name.
#
# - Written by both: its <name>.pdf and <name>_Data.xml carry one name, and
#   the four files are in the folder: <name>_Data.xml, <name>_Meta.xml,
#   <name>_Coverage.json, <name>.pdf. Otherwise the drawing does not pair.
# - Written by one only: listed, with the reason the other index gives for
#   it: the problems of its entry in _Index.json, the error of its page in
#   _PdfIndex.json. _Index.json lists no entry for a .pid it took for a border
#   template, only how many there were (template_pid_entries), and names the
#   .pid files it matched to no drawing (unmatched_pid_entries). A PDF of a
#   .pid that _Index.json does not list stands for one of those, as long as
#   there are no more such PDFs than it accounts for. One written by one only
#   with no reason does not pair.
# - Written by neither: listed, with the reasons as above.
# - A file in the folder that neither index names, or a file an index names
#   that is not in the folder, does not pair either.
#
# -ExpectPairs and -ExpectFiles, when given, are how many drawings should
# pair and how many files the folder should hold (a folder in it counts as
# one); a different number fails the check. -Report writes the same text to a
# file as well, as UTF-8. The home folder is written as %USERPROFILE%.
#
# Exit code: 0 when everything pairs; 1 when something does not, or a number
# is not the expected one; 2 when the folder or one of the indexes cannot be
# read.
#
# ASCII only: Windows PowerShell 5.1 misreads UTF-8 without a BOM.
param(
    [Parameter(Position = 0)][string]$Folder = '',
    [int]$ExpectPairs = -1,
    [int]$ExpectFiles = -1,
    [string]$Report = ''
)

$publishName = '_Index.json'
$pdfName = '_PdfIndex.json'

function Stop-Usage([string]$message) {
    [Console]::Error.WriteLine("check-pairs: $message")
    exit 2
}

function Get-Shown([string]$text) {
    $profileDir = $env:USERPROFILE
    if ($profileDir -and $text) {
        return $text.Replace($profileDir, '%USERPROFILE%')
    }
    return $text
}

function Get-Cell([string]$text) {
    return (Get-Shown $text).Replace('|', '\|').Replace("`r", ' ').Replace("`n", ' ')
}

function Read-Index([string]$path) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        Stop-Usage "$(Get-Shown $path) is not there"
    }
    try {
        $text = [IO.File]::ReadAllText($path, [Text.Encoding]::UTF8)
        $value = $text | ConvertFrom-Json -ErrorAction Stop
    } catch {
        Stop-Usage "$(Get-Shown $path) cannot be read as JSON: $($_.Exception.Message)"
    }
    if ($null -eq $value -or $value -is [array] -or $value -isnot [System.Management.Automation.PSCustomObject]) {
        Stop-Usage "$(Get-Shown $path) is not a JSON object"
    }
    return $value
}

function Get-List($value) {
    # The items of a JSON list; none for null or a missing field. Callers wrap it in @().
    if ($null -eq $value) { return }
    foreach ($item in @($value)) { $item }
}

function Get-Key([string]$path) {
    return ($path -replace '\\', '/').Trim('/').ToLowerInvariant()
}

function Remove-Suffix([string]$name, [string]$suffix) {
    if ($name.Length -gt $suffix.Length -and $name.EndsWith($suffix, [StringComparison]::OrdinalIgnoreCase)) {
        return $name.Substring(0, $name.Length - $suffix.Length)
    }
    return $null
}

if (-not $Folder) {
    Stop-Usage 'usage: check-pairs.ps1 <folder> [-ExpectPairs <n>] [-ExpectFiles <n>] [-Report <file>]'
}
if (-not (Test-Path -LiteralPath $Folder -PathType Container)) {
    Stop-Usage "$(Get-Shown $Folder) is not a folder"
}
$Folder = (Resolve-Path -LiteralPath $Folder).ProviderPath

$publish = Read-Index (Join-Path $Folder $publishName)
$pdf = Read-Index (Join-Path $Folder $pdfName)
if ($null -eq $publish.drawings) { Stop-Usage "$publishName has no drawings" }
if ($null -eq $pdf.pages) { Stop-Usage "$pdfName has no pages" }
$publishDrawings = @(Get-List $publish.drawings)
$pdfPages = @(Get-List $pdf.pages)

# What does not pair, one sentence each.
$wrong = New-Object System.Collections.Generic.List[string]

# One row per drawing, in the order of _Index.json, then the pages it does not list.
$rows = [ordered]@{}
$number = 0
foreach ($entry in $publishDrawings) {
    $number++
    $place = [string]$entry.zip_entry
    if (-not $place) {
        $wrong.Add("drawing $number of $publishName ($($entry.name)) has no zip_entry")
        continue
    }
    $key = Get-Key $place
    if ($rows.Contains($key)) {
        $wrong.Add("$publishName lists $place twice")
        continue
    }
    $rows[$key] = [pscustomobject]@{
        Place    = $place
        Name     = [string]$entry.name
        Files    = @(Get-List $entry.files | Where-Object { $_ } | ForEach-Object { [string]$_ })
        Problems = @(Get-List $entry.problems | Where-Object { $_ } | ForEach-Object { [string]$_ })
        Listed   = $true
        Page     = $null
        Result   = ''
        Reason   = ''
    }
}
$number = 0
foreach ($page in $pdfPages) {
    $number++
    $place = [string]$page.pid
    if ($place) {
        $key = Get-Key $place
    } else {
        $key = 'name:' + ([string]$page.name).ToLowerInvariant()
    }
    if ($rows.Contains($key)) {
        if ($null -ne $rows[$key].Page) {
            $wrong.Add("$pdfName has two pages for $(if ($place) { $place } else { $page.name })")
            continue
        }
        $rows[$key].Page = $page
    } else {
        $rows[$key] = [pscustomobject]@{
            Place    = $place
            Name     = [string]$page.name
            Files    = @()
            Problems = @()
            Listed   = $false
            Page     = $page
            Result   = ''
            Reason   = ''
        }
    }
}

$templates = 0
if ($null -ne $publish.template_pid_entries) { $templates = [int]$publish.template_pid_entries }
$unmatched = @(Get-List $publish.unmatched_pid_entries | ForEach-Object { Get-Key ([string]$_) })
$templatesTaken = 0

function Get-PublishReason($row) {
    # Why the publish export wrote no files of this drawing, as _Index.json says.
    if ($row.Listed) {
        if ($row.Problems.Count -gt 0) { return '_Index.json: ' + ($row.Problems -join '; ') }
        return ''
    }
    if (-not $row.Place) {
        return '_Index.json: no entry; the backup holds no .pid of it'
    }
    $key = Get-Key $row.Place
    foreach ($name in $unmatched) {
        if ($name -eq $key -or $name.EndsWith(' ' + $key) -or $name.EndsWith('/' + $key)) {
            return '_Index.json: in unmatched_pid_entries, matched to no drawing'
        }
    }
    if ($script:templatesTaken -lt $templates) {
        $script:templatesTaken++
        return "_Index.json: no entry; one of the $templates .pid it took for border templates (template_pid_entries)"
    }
    return ''
}

function Get-PdfReason($row) {
    # Why the PDF export wrote no PDF of this drawing, as _PdfIndex.json says.
    if ($null -eq $row.Page) { return '' }
    $said = [string]$row.Page.error
    if ($said) { return "_PdfIndex.json: $said" }
    return ''
}

# The files in the folder, the ones the indexes name, and the ones a pair was checked for.
$entries = @(Get-ChildItem -LiteralPath $Folder -Force)
$present = New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
foreach ($item in $entries) { [void]$present.Add($item.Name) }
$named = New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
[void]$named.Add($publishName)
[void]$named.Add($pdfName)
$checked = New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)

$both = 0
$paired = 0
$publishOnly = 0
$pdfOnly = 0
$neither = 0
foreach ($row in $rows.Values) {
    $files = $row.Files
    foreach ($file in $files) { [void]$named.Add($file) }
    $pdfFile = ''
    if ($null -ne $row.Page -and $row.Page.pdf) { $pdfFile = [string]$row.Page.pdf }
    if ($pdfFile) { [void]$named.Add($pdfFile) }
    $shown = if ($row.Place) { $row.Place } else { $row.Name }

    if ($files.Count -gt 0 -and $pdfFile) {
        $both++
        $data = @($files | Where-Object { $_.EndsWith('_Data.xml', [StringComparison]::OrdinalIgnoreCase) })
        $stem = $null
        if ($data.Count -eq 1) { $stem = Remove-Suffix $data[0] '_Data.xml' }
        $pdfStem = Remove-Suffix $pdfFile '.pdf'
        if ($null -eq $stem) {
            $row.Result = 'does not pair'
            $row.Reason = "_Index.json names $($data.Count) _Data.xml files for it"
        } elseif ($null -eq $pdfStem -or $pdfStem -cne $stem) {
            $row.Result = 'does not pair'
            $row.Reason = "$pdfFile and $($data[0]) do not carry one name"
        } else {
            $missing = @()
            foreach ($want in @("$($stem)_Data.xml", "$($stem)_Meta.xml", "$($stem)_Coverage.json")) {
                if (-not ($files -ccontains $want)) { $missing += "$want (not in _Index.json)" }
            }
            foreach ($want in @($files + $pdfFile)) {
                [void]$checked.Add($want)
                if (-not $present.Contains($want)) { $missing += "$want (not in the folder)" }
            }
            if ($missing.Count -gt 0) {
                $row.Result = 'does not pair'
                $row.Reason = 'missing ' + ($missing -join ', ')
            } else {
                $row.Result = 'paired'
                $paired++
            }
        }
        if ($row.Result -ne 'paired') { $wrong.Add("$shown does not pair: $($row.Reason)") }
    } elseif ($files.Count -gt 0) {
        $publishOnly++
        $row.Result = 'publish only'
        if ($null -eq $row.Page) {
            $wrong.Add("$shown has publish files and no page in $pdfName")
        } else {
            $row.Reason = Get-PdfReason $row
            if (-not $row.Reason) { $wrong.Add("$shown has publish files and no PDF, and $pdfName gives no reason") }
        }
    } elseif ($pdfFile) {
        $pdfOnly++
        $row.Result = 'PDF only'
        $row.Reason = Get-PublishReason $row
        if (-not $row.Reason) { $wrong.Add("$shown has a PDF and no publish files, and $publishName gives no reason") }
    } else {
        $neither++
        $row.Result = 'neither'
        $row.Reason = @((Get-PublishReason $row), (Get-PdfReason $row) | Where-Object { $_ }) -join ' / '
    }
}

foreach ($name in @($named | Sort-Object)) {
    if (-not $present.Contains($name) -and -not $checked.Contains($name)) {
        $wrong.Add("$name is named by an index and is not in the folder")
    }
}
foreach ($name in @($entries | Where-Object { -not $named.Contains($_.Name) } | ForEach-Object { $_.Name } | Sort-Object)) {
    $wrong.Add("$name is in the folder and neither index names it")
}

$fileCount = @($entries | Where-Object { -not $_.PSIsContainer }).Count
if ($ExpectPairs -ge 0 -and $paired -ne $ExpectPairs) {
    $wrong.Add("$paired drawings pair, $ExpectPairs expected")
}
if ($ExpectFiles -ge 0 -and $entries.Count -ne $ExpectFiles) {
    $wrong.Add("the folder holds $($entries.Count), $ExpectFiles expected")
}

$lines = New-Object System.Collections.Generic.List[string]
$lines.Add("check-pairs: $(Get-Shown $Folder)")
$lines.Add("package in $($publishName): $(Get-Shown ([string]$publish.package))")
$lines.Add("package in $($pdfName): $(Get-Shown ([string]$pdf.package))")
$lines.Add(("$($publishName): export {0}, pid_entries {1}, drawing_rows {2}, paired {3}, failed {4}, template_pid_entries {5}, unmatched_pid_entries {6}, drawings listed {7}" -f
        $publish.export, $publish.pid_entries, $publish.drawing_rows, $publish.paired, $publish.failed, $templates,
        $unmatched.Count, $publishDrawings.Count))
$lines.Add(("$($pdfName): drawings {0}, written {1}, failed {2}, without_file {3}, pages listed {4}" -f
        $pdf.drawings, $pdf.written, $pdf.failed, $pdf.without_file, $pdfPages.Count))
$lines.Add("drawings: $($rows.Count); written by both $both, of them paired $paired; publish only $publishOnly; PDF only $pdfOnly; neither $neither")
$lines.Add("folder: $($entries.Count) entries, $fileCount of them files; the indexes name $($named.Count) files, the two indexes with them")
$lines.Add('')
$lines.Add('| # | Drawing | .pid in the package | Publish files | PDF | Result | Reason |')
$lines.Add('|---:|---|---|---:|---|---|---|')
$number = 0
foreach ($row in $rows.Values) {
    $number++
    $pdfCell = ''
    if ($null -ne $row.Page -and $row.Page.pdf) { $pdfCell = [string]$row.Page.pdf }
    $lines.Add(("| {0} | {1} | {2} | {3} | {4} | {5} | {6} |" -f $number, (Get-Cell $row.Name), (Get-Cell $row.Place),
            $row.Files.Count, (Get-Cell $pdfCell), $row.Result, (Get-Cell $row.Reason)))
}
$lines.Add('')
if ($wrong.Count -eq 0) {
    $lines.Add("PASS: $paired drawings pair; $publishOnly publish only and $pdfOnly PDF only, each with its reason; $neither neither")
} else {
    $lines.Add("FAIL: $($wrong.Count) thing(s) do not pair:")
    foreach ($sentence in $wrong) { $lines.Add("- $(Get-Shown $sentence)") }
}

foreach ($line in $lines) { Write-Output $line }
if ($Report) {
    [IO.File]::WriteAllLines($Report, $lines, (New-Object System.Text.UTF8Encoding $false))
}
if ($wrong.Count -eq 0) { exit 0 }
exit 1
