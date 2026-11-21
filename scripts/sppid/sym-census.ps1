# Runs repeatable .sym library decoding census (#114) and .pid missing symbol
# inventory (#113) via pid-parse's native Rust decoders.
#
# Usage:
#   scripts\sppid\sym-census.ps1 [-SymSources "label1=C:\path1;label2=C:\path2"]
#                                [-PidDir "C:\pids"] [-SymLib "C:\syms"]
#                                [-ScopeNote "..."] [-Out "D:\scratch\out"]
#
# ASCII only: Windows PowerShell 5.1 misreads UTF-8 without a BOM.
param(
    [string]$SymSources = '',
    [string]$PidDir = '',
    [string]$SymLib = '',
    [string]$ScopeNote = '',
    [string]$Out = ''
)

$ErrorActionPreference = 'Stop'

if (-not $SymSources -and $env:OCS_PID_CORPUS_DIR) {
    $defaultSym = Join-Path $env:OCS_PID_CORPUS_DIR 'symbols'
    if (Test-Path $defaultSym) {
        $SymSources = "corpus=$defaultSym"
        if (-not $SymLib) { $SymLib = $defaultSym }
    }
}

if (-not $PidDir -and $env:OCS_PID_CORPUS_DIR) {
    $defaultExport = Join-Path $env:OCS_PID_CORPUS_DIR 'export-set'
    if (Test-Path $defaultExport) {
        $PidDir = $defaultExport
    }
}

if (-not $SymSources -and -not $PidDir) {
    Write-Output 'OCS_PID_CORPUS_DIR not set and no -SymSources/-PidDir given; skipping.'
    exit 0
}

if (-not $Out) {
    if ($env:OCS_PID_EVIDENCE_DIR) {
        $Out = $env:OCS_PID_EVIDENCE_DIR
    } else {
        $stamp = Get-Date -Format 'yyyy-MM-dd'
        $Out = Join-Path ([System.IO.Path]::GetTempPath()) "ocs-sym-audit-$stamp"
    }
}

New-Item -ItemType Directory -Force $Out | Out-Null

$repoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Push-Location $repoRoot
try {
    if ($SymSources) {
        $env:OCS_PID_SYM_CENSUS_SOURCES = $SymSources
        $env:OCS_PID_SYM_CENSUS_OUT = $Out
        cargo test --offline --locked --lib io::pid_sym_census::tests::real_corpus_sym_census -- --ignored --nocapture
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    }

    if ($PidDir -and $SymLib) {
        $env:OCS_PID_SYM_AUDIT_PIDS = $PidDir
        $env:OCS_PID_SYM_AUDIT_LIB = $SymLib
        $env:OCS_PID_SYM_AUDIT_NOTE = $ScopeNote
        $env:OCS_PID_SYM_AUDIT_OUT = (Join-Path $Out 'sym-missing')
        cargo test --offline --locked --lib io::pid_sym_missing::tests::real_corpus_symbol_missing_inventory -- --ignored --nocapture
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    }
} finally {
    Remove-Item Env:OCS_PID_SYM_CENSUS_SOURCES -ErrorAction SilentlyContinue
    Remove-Item Env:OCS_PID_SYM_CENSUS_OUT -ErrorAction SilentlyContinue
    Remove-Item Env:OCS_PID_SYM_AUDIT_PIDS -ErrorAction SilentlyContinue
    Remove-Item Env:OCS_PID_SYM_AUDIT_LIB -ErrorAction SilentlyContinue
    Remove-Item Env:OCS_PID_SYM_AUDIT_NOTE -ErrorAction SilentlyContinue
    Remove-Item Env:OCS_PID_SYM_AUDIT_OUT -ErrorAction SilentlyContinue
    Pop-Location
}
exit 0
