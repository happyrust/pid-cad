<#
.SYNOPSIS
    P5.3 / P8.3 of the SVG export plans: render the pages this tree writes
    with the renderers people actually open SVGs with, then judge every
    rendering against resvg with the tree's own raster comparator.

.DESCRIPTION
    Three steps, each skippable:

    1. Samples (-SkipSamples to reuse what is in -SamplesDir):
         corpus\  the plot corpus (`cargo test --lib dump_the_corpus_for_a_human`
                  writes %TEMP%\ocs-svg-corpus\*.svg, copied in as corpus-*.svg)
                  plus the merge_lines minimal pair from the raster evidence;
         real\    the real sheets: the debug OpenCADStudio.exe --plot-svg on each
                  DXF named by -Sheets in -SheetsDir (--model --paper A1
                  --landscape --fit, the page setup's own style table — the same
                  plot the raster evidence took), written as real-<code>.svg.
    2. Renderings: every <stem>.svg goes through each renderer in -Renderers
       that is on this machine, on white, to <stem>.<renderer>.png — corpus\
       at -Dpi (600 by default, the raster evidence's dpi, where the comparator
       is calibrated: at 150 dpi a 0.75 pt line is 1.5 px and every renderer's
       anti-aliasing quantisation trips it), real\ at -RealDpi (254 by default
       = 10 px/mm: an A1 sheet comes out a whole 8410 × 5940, so no renderer
       has to stretch the page to whole pixels — Inkscape and librsvg do, and
       at 300 dpi that 0.004 % stretch alone fails every tile along the bottom
       frame line; and an A1 at 600 is a 1.1 GB pixmap per engine).
       -Only corpus|real restricts a run to one set.
         chromium  Google Chrome or Microsoft Edge, headless --screenshot; the
                   window is whole CSS px, so the shot is cropped to the page.
         inkscape  inkscape.com --export-type=png (winget Inkscape.Inkscape)
         rsvg      librsvg, as bundled in libvips through the sharp npm
                   package; installed under %LOCALAPPDATA%\ocs-svg-compat\node
                   on first use (-NoInstall forbids that). Nothing enters the
                   tree's dependency graph.
       Versions land in <SamplesDir>\versions.txt; a renderer that is missing
       is recorded as missing and leaves its column empty — never a fake ok.
    3. Comparison (-SkipCompare to stop before it): the ignored test
       `compare_external_renders` renders each SVG with resvg at the same dpi
       and compares every external PNG with the P5.2 comparator, writing
       <sub>\compat-<dpi>dpi.tsv and diff maps under <sub>\diff. It runs
       --release (the debug comparator takes seconds per megapixel);
       -DebugProfile keeps it in the dev profile.

    Run from anywhere; the repository is the script's parent folder.

.EXAMPLE
    pwsh scripts\svg-compat.ps1
    pwsh scripts\svg-compat.ps1 -SkipSamples -Renderers chromium -Dpi 150 -RealDpi 150
#>
[CmdletBinding()]
param(
    [string]$SamplesDir = (Join-Path $env:TEMP 'ocs-svg-compat'),
    [string]$SheetsDir = '',
    [double]$Dpi = 600,
    [double]$RealDpi = 254,
    [ValidateSet('corpus', 'real', 'both')]
    [string]$Only = 'both',
    [ValidateSet('chromium', 'inkscape', 'rsvg')]
    [string[]]$Renderers = @('chromium', 'inkscape', 'rsvg'),
    [string[]]$Sheets = @('FF02-06', 'SP02-05', 'WS02-05'),
    [string]$SharpVersion = '0.35.4',
    [switch]$SkipSamples,
    [switch]$SkipCompare,
    [switch]$NoInstall,
    [switch]$DebugProfile,
    [switch]$Force
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
if (-not $SheetsDir) { $SheetsDir = Join-Path (Split-Path -Parent $repo) '0版重新处理dxf-12张' }
$evidence = Join-Path $repo 'docs\evidence\2026-09-09-svg-pdf-raster'
$corpusOut = Join-Path $SamplesDir 'corpus'
$realOut = Join-Path $SamplesDir 'real'
New-Item -ItemType Directory -Force $corpusOut, $realOut | Out-Null
$versions = [ordered]@{ resvg = '0.45.1 (the tree''s dev-dependency; the reference)' }
Add-Type -AssemblyName System.Drawing

function Step($text) { Write-Host "`n== $text" -ForegroundColor Cyan }

# ── 1. samples ──────────────────────────────────────────────────────────────

if (-not $SkipSamples) {
    Step 'corpus pages'
    Push-Location $repo
    try {
        # The dump appends to a fixed folder; clear it, or pages the corpus no
        # longer has ride along under stale names.
        $corpusDir = Join-Path $env:TEMP 'ocs-svg-corpus'
        if (Test-Path $corpusDir) { Remove-Item $corpusDir -Recurse -Force }
        cargo test --lib dump_the_corpus_for_a_human -- --ignored --nocapture 2>&1 |
            Select-String -Pattern '^wrote |test result' | ForEach-Object Line
        Get-ChildItem $corpusDir -Filter *.svg | ForEach-Object {
            Copy-Item $_.FullName (Join-Path $corpusOut "corpus-$($_.Name)") -Force
        }
        "copied $((Get-ChildItem $corpusDir -Filter *.svg).Count) corpus pages"
        Get-ChildItem $evidence -Filter 'merge-lines-minimal-*.svg' | ForEach-Object {
            Copy-Item $_.FullName (Join-Path $corpusOut $_.Name) -Force; "  + $($_.Name)"
        }

        Step 'real sheets'
        if (-not (Test-Path $SheetsDir)) {
            Write-Warning "no sheet folder at $SheetsDir (pass -SheetsDir); real sheets skipped"
        } else {
            # The target directory is wherever cargo says (CARGO_TARGET_DIR is set on some desks).
            $targetDir = (cargo metadata --format-version 1 --no-deps 2>$null | ConvertFrom-Json).target_directory
            $exe = Join-Path $targetDir 'debug\OpenCADStudio.exe'
            if (-not (Test-Path $exe)) { cargo build 2>&1 | Select-Object -Last 1 }
            foreach ($code in $Sheets) {
                $dxf = Get-ChildItem $SheetsDir -Filter "DWG-0100$code *.dxf" | Select-Object -First 1
                if (-not $dxf) { Write-Warning "no DXF for $code in $SheetsDir"; continue }
                $svg = Join-Path $realOut "real-$code.svg"
                & $exe --plot-svg $dxf.FullName $svg --model --paper A1 --landscape --fit --force 2>&1 |
                    ForEach-Object { "  $code : $_" }
            }
        }
    } finally { Pop-Location }
}

# The page size the SVG states, in millimetres, from its root element.
function PageMm($svgPath) {
    $head = [IO.File]::ReadAllText($svgPath).Substring(0, 400)
    if ($head -match 'width="([0-9.]+)mm"\s+height="([0-9.]+)mm"') {
        return @([double]$Matches[1], [double]$Matches[2])
    }
    throw "$svgPath does not state its size in mm"
}

# Keep the top-left w×h of a PNG (in place) when it is larger than that.
function CropTopLeft($png, $w, $h) {
    $img = [System.Drawing.Bitmap]::new($png)
    try {
        if ($img.Width -le $w -and $img.Height -le $h) { return }
        $w = [math]::Min($w, $img.Width); $h = [math]::Min($h, $img.Height)
        $out = $img.Clone([System.Drawing.Rectangle]::new(0, 0, [int]$w, [int]$h), $img.PixelFormat)
    } finally { $img.Dispose() }
    try { $out.Save("$png.tmp", [System.Drawing.Imaging.ImageFormat]::Png) } finally { $out.Dispose() }
    Move-Item "$png.tmp" $png -Force
}

# ── 2. renderers ────────────────────────────────────────────────────────────

$browser = @(
    "$env:ProgramFiles\Google\Chrome\Application\chrome.exe",
    "${env:ProgramFiles(x86)}\Google\Chrome\Application\chrome.exe",
    "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe",
    "$env:ProgramFiles\Microsoft\Edge\Application\msedge.exe"
) | Where-Object { Test-Path $_ } | Select-Object -First 1
$inkscape = @(
    (Get-Command inkscape.com -ErrorAction SilentlyContinue | ForEach-Object Source),
    "$env:ProgramFiles\Inkscape\bin\inkscape.com"
) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
$nodeDir = Join-Path $env:LOCALAPPDATA 'ocs-svg-compat\node'
$sharpReady = $false
if ($Renderers -contains 'rsvg' -and (Get-Command node -ErrorAction SilentlyContinue)) {
    if (-not (Test-Path (Join-Path $nodeDir 'node_modules\sharp')) -and -not $NoInstall) {
        New-Item -ItemType Directory -Force $nodeDir | Out-Null
        Push-Location $nodeDir
        try {
            if (-not (Test-Path package.json)) { npm init -y 2>&1 | Out-Null }
            npm install "sharp@$SharpVersion" --no-audit --no-fund 2>&1 | Select-Object -Last 1
        } finally { Pop-Location }
    }
    $sharpReady = Test-Path (Join-Path $nodeDir 'node_modules\sharp')
}
# A page stated in mm meets sharp's `density` twice: librsvg lays the mm out at
# `density` px per inch, then libvips scales that picture by density / 72 again
# (measured: 72 → 72 dpi, 96 → 128, 150 → 312.5). So the density that lands on
# `dpi` device pixels per inch is sqrt(72 × dpi) — 103.92 for 150 dpi, which
# gives resvg's 1754 × 1240 for an A4 landscape page exactly.
$rsvgScript = @'
const fs = require('fs'), path = require('path');
const sharp = require(path.join(process.argv[2], 'node_modules', 'sharp'));
const pkg = JSON.parse(fs.readFileSync(path.join(process.argv[2], 'node_modules', 'sharp', 'package.json')));
const dpi = Number(process.argv[3]);
const density = Math.sqrt(72 * dpi);
const jobs = process.argv.slice(4);
(async () => {
  if (jobs.length === 0) {
    const v = sharp.versions;
    console.log(`librsvg ${v.rsvg} via libvips ${v.vips} / sharp ${pkg.version} (cairo ${v.cairo}; density sqrt(72 × dpi) for a mm page)`);
    return;
  }
  for (let i = 0; i < jobs.length; i += 2) {
    const [svg, png] = [jobs[i], jobs[i + 1]];
    await sharp(svg, { density, limitInputPixels: false })
      .flatten({ background: '#ffffff' }).png({ compressionLevel: 6 }).toFile(png);
    console.log(`  ${path.basename(svg, '.svg')} -> rsvg`);
  }
})().catch(e => { console.error(e.message); process.exit(1); });
'@

function RenderAll($dir, $dpi) {
    $samples = Get-ChildItem $dir -Filter *.svg | Sort-Object Name
    if (-not $samples) { Write-Warning "no .svg samples in $dir"; return }
    Step "$($samples.Count) samples in $dir at $dpi dpi"
    function Wanted($svg, $renderer) {
        $png = Join-Path $dir "$($svg.BaseName).$renderer.png"
        if ((Test-Path $png) -and -not $Force) { return $null }
        return $png
    }

    if ($Renderers -contains 'chromium') {
        if (-not $browser) {
            Write-Warning 'no Chrome or Edge found; chromium column left empty'
            $script:versions.chromium = 'MISSING'
        } else {
            $info = (Get-Item $browser).VersionInfo
            $script:versions.chromium = "$($info.ProductName) $($info.ProductVersion) (headless=new --screenshot, --force-device-scale-factor = dpi / 96, cropped to the page)"
            $profile = Join-Path $env:TEMP 'ocs-svg-compat-chromium-profile'
            foreach ($svg in $samples) {
                $png = Wanted $svg 'chromium'; if (-not $png) { continue }
                $mm = PageMm $svg.FullName
                # The window is whole CSS px (96 per inch) and the shot is window ×
                # scale factor; the page itself is cropped out of it afterwards, at
                # the size resvg makes of the same dpi.
                $w = [math]::Ceiling($mm[0] * 96 / 25.4); $h = [math]::Ceiling($mm[1] * 96 / 25.4)
                $pw = [math]::Ceiling($mm[0] / 25.4 * $dpi); $ph = [math]::Ceiling($mm[1] / 25.4 * $dpi)
                $url = 'file:///' + ($svg.FullName -replace '\\', '/')
                # Headless Chromium leaves raster tiles beyond its default tile
                # memory budget (~256 MB) unpainted — white, no error: an A1 sheet
                # at 300 dpi is 279 MB and lost everything below row 6351 of 7016.
                # The budget flag lifts it; measured full at 4096.
                & $browser --headless=new --disable-gpu --hide-scrollbars --no-first-run --no-default-browser-check `
                    --run-all-compositor-stages-before-draw --force-gpu-mem-available-mb=4096 --user-data-dir="$profile" `
                    --force-device-scale-factor=$($dpi / 96) --window-size="$w,$h" --default-background-color=ffffffff `
                    "--screenshot=$png" $url 2>&1 | Out-Null
                if (Test-Path $png) { CropTopLeft $png $pw $ph; "  $($svg.BaseName) -> chromium" }
                else { Write-Warning "  $($svg.BaseName): chromium wrote nothing" }
            }
        }
    }

    if ($Renderers -contains 'inkscape') {
        if (-not $inkscape) {
            Write-Warning 'no Inkscape found (winget install Inkscape.Inkscape); inkscape column left empty'
            $script:versions.inkscape = 'MISSING'
        } else {
            $script:versions.inkscape = ((& $inkscape --version 2>&1 | Select-Object -First 1) -join '') + ' (--export-type=png --export-dpi)'
            foreach ($svg in $samples) {
                $png = Wanted $svg 'inkscape'; if (-not $png) { continue }
                & $inkscape --export-type=png --export-dpi=$dpi --export-background=white --export-background-opacity=1 `
                    "--export-filename=$png" $svg.FullName 2>&1 | Where-Object { $_ -match 'error|Error' } | ForEach-Object { Write-Warning "  $_" }
                if (Test-Path $png) { "  $($svg.BaseName) -> inkscape" } else { Write-Warning "  $($svg.BaseName): inkscape wrote nothing" }
            }
        }
    }

    if ($Renderers -contains 'rsvg') {
        if (-not $sharpReady) {
            Write-Warning 'librsvg (sharp) is not available; rsvg column left empty'
            $script:versions.rsvg = 'MISSING (node or sharp; see -NoInstall)'
        } else {
            $script:versions.rsvg = ($rsvgScript | node - $nodeDir $dpi) -join ''
            $jobs = @()
            foreach ($svg in $samples) {
                $png = Wanted $svg 'rsvg'; if (-not $png) { continue }
                $jobs += $svg.FullName; $jobs += $png
            }
            if ($jobs) { $rsvgScript | node - $nodeDir $dpi @jobs }
        }
    }
}

$sets = @()
if ($Only -ne 'real') { $sets += , @($corpusOut, $Dpi) }
if ($Only -ne 'corpus') { $sets += , @($realOut, $RealDpi) }
foreach ($pair in $sets) { RenderAll $pair[0] $pair[1] }

$versionsFile = Join-Path $SamplesDir 'versions.txt'
($versions.GetEnumerator() | ForEach-Object { "$($_.Key)`t$($_.Value)" }) -join "`n" |
    Set-Content -Path $versionsFile -Encoding UTF8
Step "versions ($versionsFile)"
Get-Content $versionsFile

# ── 3. comparison ───────────────────────────────────────────────────────────

if (-not $SkipCompare) {
    foreach ($pair in $sets) {
        $dir, $dpi = $pair
        if (-not (Get-ChildItem $dir -Filter *.svg)) { continue }
        Step "comparison against resvg: $dir at $dpi dpi"
        Push-Location $repo
        try {
            $env:OCS_SVG_COMPAT_DIR = $dir
            $env:OCS_SVG_COMPAT_DPI = "$dpi"
            $lines = if ($DebugProfile) {
                cargo test --lib compare_external_renders -- --ignored --nocapture 2>&1
            } else {
                cargo test --release --lib compare_external_renders -- --ignored --nocapture 2>&1
            }
            $lines | Select-String -Pattern '^compared |test result|panicked|^error' | ForEach-Object Line
        } finally {
            Remove-Item Env:\OCS_SVG_COMPAT_DIR, Env:\OCS_SVG_COMPAT_DPI -ErrorAction SilentlyContinue
            Pop-Location
        }
        $table = Join-Path $dir "compat-${dpi}dpi.tsv"
        if (Test-Path $table) { Step $table; Get-Content $table }
    }
}
