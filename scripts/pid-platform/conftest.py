"""Shared paths and helpers for the P&ID platform documentation contract tests.

Every check in this package must assert real filesystem state.
"""
from __future__ import annotations

import json
import re
import sys
from pathlib import Path

import pytest

# manifest_contract is the single implementation of heading normalization;
# conftest re-exports it so `from conftest import normalize_heading` works.
sys.path.insert(0, str(Path(__file__).resolve().parent))
from manifest_contract import headings, normalize_heading  # noqa: E402,F401

REPO_ROOT = Path(__file__).resolve().parents[2]
DOCS = REPO_ROOT / "docs" / "pid-platform"
PROTO = DOCS / "prototype"
CATALOG = PROTO / "specs" / "screen-catalog.json"
MANIFEST = DOCS / "manifest.json"

# Normative promised deliverable set (see VAL-DOC-008).
EXPECTED_DELIVERABLES = [
    "README.md",
    "PRD.md",
    "glossary.md",
    "gap-analysis.md",
    "reference/source-index.md",
    "reference/reference-product-summary.md",
    "architecture/system-architecture.md",
    "architecture/data-model.md",
    "architecture/non-functional.md",
    "requirements/00-overview.md",
    "requirements/10-platform.md",
    "requirements/20-type-library.md",
    "requirements/30-symbol-library-and-project-settings.md",
    "requirements/40-drawing-and-drafting.md",
    "requirements/50-data-and-rule-engine.md",
    "requirements/60-deliverables-and-publish.md",
    "requirements/70-collaboration-and-permissions.md",
    "requirements/80-import-export-matrix.md",
    "requirements/90-migration-and-interop.md",
    "requirements/95-reuse-mapping.md",
]

SCREEN_IDS = [
    "workbench", "drawings", "editor", "backup", "palette", "properties", "typelib",
    "symbols", "symbol-editor", "tag-rules", "color-rules", "masterdata",
    "validation", "bom", "permissions",
]

REQ_AREA_FILES = {
    "PLAT": "10-platform.md",
    "TYPE": "20-type-library.md",
    "SYM": "30-symbol-library-and-project-settings.md",
    "DRW": "40-drawing-and-drafting.md",
    "DATA": "50-data-and-rule-engine.md",
    "DELV": "60-deliverables-and-publish.md",
    "COLL": "70-collaboration-and-permissions.md",
    "IO": "80-import-export-matrix.md",
    "MIG": "90-migration-and-interop.md",
}

# Anchored so VAL-REQ-001 style assertion IDs are never matched.
REQ_ID_RE = re.compile(
    r"(?<![A-Za-z-])REQ-(PLAT|TYPE|SYM|DRW|DATA|DELV|COLL|IO|MIG)-\d{3}(?!\d)"
)


def load_manifest() -> dict:
    with MANIFEST.open(encoding="utf-8") as fh:
        return json.load(fh)


def delivered_entries(manifest: dict) -> list:
    return [d for d in manifest.get("deliverables", []) if d.get("status") == "delivered"]


@pytest.fixture(scope="session")
def manifest() -> dict:
    if not MANIFEST.exists():
        pytest.fail("manifest.json missing at " + str(MANIFEST))
    return load_manifest()


@pytest.fixture(scope="session")
def catalog() -> dict:
    if not CATALOG.exists():
        pytest.fail("screen-catalog.json missing at " + str(CATALOG))
    with CATALOG.open(encoding="utf-8") as fh:
        return json.load(fh)
