#!/usr/bin/env python3
"""Index and verify P&ID phase-1 corpus manifests (L0-2 / #107).

Standard-library CLI and helper module that binds phase-1 report metrics to
exact external sample files via relative paths and SHA-256 digests, without
ever recording machine-specific absolute paths in `corpus.manifest.json`.

Exit codes:
- 0: success, or `OCS_PID_CORPUS_DIR` is unset when `--dir` is omitted;
- 1: `verify` detected missing, unexpected, or modified files / bad digest;
- 2: invalid CLI arguments, invalid path mapping, or unreadable directory.
"""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import sys
from typing import Any, Sequence

CORPUS_ENV = "OCS_PID_CORPUS_DIR"
MANIFEST_FILENAME = "corpus.manifest.json"
SCHEMA_VERSION = 1

DEFAULT_LAYOUT: dict[str, str] = {
    "EXPORT_SET": "export-set",
    "EXPORT_DXF": "export-dxf",
    "EXPORT_PDF": "export-pdf",
    "SOURCE_DWG": "source-dwg",
    "BACKUP_ORACLE": "backup-oracle",
    "BACKUP_TABLES": "backup-tables",
    "SYMBOLS": "symbols",
    "SEEDS": "seeds",
    "EXTERNAL": "external-platform",
    "CONTROLLED": "controlled",
}


def _ensure_utf8_stdio() -> None:
    for stream in (sys.stdout, sys.stderr):
        enc = (getattr(stream, "encoding", "") or "").lower().replace("-", "")
        if enc != "utf8" and hasattr(stream, "reconfigure"):
            try:
                stream.reconfigure(encoding="utf-8", errors="replace")
            except Exception:
                pass


def validate_rel_posix_path(raw: str, *, allow_slash_convert: bool = False) -> str:
    """Validate and normalize a strictly relative POSIX path under corpus root."""
    if not raw or not raw.strip():
        raise ValueError("empty relative path is not allowed")
    text = raw.strip()
    if allow_slash_convert:
        text = text.replace("\\", "/")
    elif "\\" in text:
        raise ValueError(f"backslash is not allowed in manifest path: {raw!r}")
    if text.startswith("/") or ":" in text:
        raise ValueError(f"absolute or drive-qualified path is forbidden: {raw!r}")
    parts = [part for part in text.split("/") if part != ""]
    if not parts:
        raise ValueError(f"invalid relative path: {raw!r}")
    for part in parts:
        if part in (".", ".."):
            raise ValueError(f"dot or parent traversal segment is forbidden: {raw!r}")
    return "/".join(parts)


def sha256_file(path: Path) -> tuple[int, str]:
    hasher = hashlib.sha256()
    total = 0
    with path.open("rb") as fh:
        while True:
            chunk = fh.read(1024 * 1024)
            if not chunk:
                break
            total += len(chunk)
            hasher.update(chunk)
    return total, hasher.hexdigest()


def compute_manifest_digest(sections: dict[str, Any]) -> str:
    canonical = json.dumps(
        {"schema_version": SCHEMA_VERSION, "sections": sections},
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")
    return hashlib.sha256(canonical).hexdigest()


def _scan_section(
    corpus_root: Path,
    subdir: str,
    manifest_resolved: Path | None,
) -> dict[str, Any]:
    target = (corpus_root / Path(*subdir.split("/"))).resolve()
    root_resolved = corpus_root.resolve()
    try:
        target.relative_to(root_resolved)
    except ValueError as exc:
        raise ValueError(f"mapped directory escapes corpus root: {subdir}") from exc

    if not target.exists():
        return {
            "subdir": subdir,
            "present": False,
            "file_count": 0,
            "total_bytes": 0,
            "files": [],
        }
    if not target.is_dir():
        raise ValueError(f"mapped section path is not a directory: {subdir}")

    files: list[dict[str, Any]] = []
    total_bytes = 0
    all_files = sorted(
        (p for p in target.rglob("*") if p.is_file()),
        key=lambda p: p.relative_to(target).as_posix(),
    )
    for file_path in all_files:
        if manifest_resolved is not None and file_path.resolve() == manifest_resolved:
            continue
        rel_posix = validate_rel_posix_path(file_path.relative_to(target).as_posix())
        size, digest = sha256_file(file_path)
        total_bytes += size
        files.append(
            {
                "path": rel_posix,
                "bytes": size,
                "sha256": digest,
            }
        )

    return {
        "subdir": subdir,
        "present": True,
        "file_count": len(files),
        "total_bytes": total_bytes,
        "files": files,
    }


def _parse_mappings(raw_maps: Sequence[str]) -> dict[str, str]:
    overrides: dict[str, str] = {}
    for item in raw_maps:
        if "=" not in item:
            raise ValueError(f"invalid --map {item!r}: expected KEY=REL_SUBDIR")
        key, raw_sub = item.split("=", 1)
        key = key.strip()
        if not key:
            raise ValueError(f"invalid --map {item!r}: empty logical key")
        overrides[key] = validate_rel_posix_path(raw_sub, allow_slash_convert=True)
    return overrides


def load_manifest(manifest_path: Path) -> dict[str, Any]:
    data = json.loads(manifest_path.read_text(encoding="utf-8"))
    if not isinstance(data, dict) or data.get("schema_version") != SCHEMA_VERSION:
        raise ValueError(f"unsupported manifest schema in {manifest_path}")
    sections = data.get("sections")
    if not isinstance(sections, dict):
        raise ValueError("manifest missing 'sections' object")
    for sec_name, sec in sections.items():
        if not isinstance(sec, dict):
            raise ValueError(f"invalid section {sec_name!r}")
        validate_rel_posix_path(str(sec.get("subdir", "")))
        for entry in sec.get("files", []):
            validate_rel_posix_path(str(entry.get("path", "")))
    return data


def resolve_section_files(
    corpus_dir: Path,
    section_name: str,
    manifest_path: Path | None = None,
) -> list[Path]:
    """Return resolved file paths for `section_name` using manifest or default layout."""
    mpath = manifest_path or (corpus_dir / MANIFEST_FILENAME)
    if mpath.is_file():
        manifest = load_manifest(mpath)
        sec = manifest.get("sections", {}).get(section_name)
        if sec is None:
            return []
        subdir = sec["subdir"]
        base = corpus_dir / Path(*subdir.split("/"))
        return [base / Path(*entry["path"].split("/")) for entry in sec.get("files", [])]
    subdir = DEFAULT_LAYOUT.get(section_name)
    if not subdir:
        return []
    base = corpus_dir / Path(*subdir.split("/"))
    if not base.is_dir():
        return []
    return sorted(
        (p for p in base.rglob("*") if p.is_file()),
        key=lambda p: p.relative_to(base).as_posix(),
    )


def build_manifest(
    corpus_dir: Path,
    *,
    manifest_path: Path,
    overrides: dict[str, str],
    reset_layout: bool = False,
) -> dict[str, Any]:
    layout = dict(DEFAULT_LAYOUT)
    if not reset_layout and manifest_path.is_file():
        existing = load_manifest(manifest_path)
        for key, sec in existing.get("sections", {}).items():
            if isinstance(sec, dict) and "subdir" in sec:
                layout[key] = validate_rel_posix_path(str(sec["subdir"]))
    layout.update(overrides)

    manifest_resolved = manifest_path.resolve()
    sections: dict[str, Any] = {}
    for key in layout:
        sections[key] = _scan_section(corpus_dir, layout[key], manifest_resolved)

    manifest_sha = compute_manifest_digest(sections)
    total_files = sum(sec["file_count"] for sec in sections.values())
    total_bytes = sum(sec["total_bytes"] for sec in sections.values())
    return {
        "schema_version": SCHEMA_VERSION,
        "generated_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "manifest_sha256": manifest_sha,
        "total_files": total_files,
        "total_bytes": total_bytes,
        "sections": sections,
    }


def verify_manifest(corpus_dir: Path, manifest_path: Path) -> tuple[bool, list[str], dict[str, Any]]:
    manifest = load_manifest(manifest_path)
    sections = manifest["sections"]
    errors: list[str] = []

    expected_manifest_sha = manifest.get("manifest_sha256")
    actual_manifest_sha = compute_manifest_digest(sections)
    if expected_manifest_sha != actual_manifest_sha:
        errors.append(
            f"MANIFEST_DIGEST_MISMATCH expected={expected_manifest_sha} actual={actual_manifest_sha}"
        )

    manifest_resolved = manifest_path.resolve()
    root_resolved = corpus_dir.resolve()

    for sec_name, sec in sections.items():
        subdir = validate_rel_posix_path(str(sec["subdir"]))
        target = (corpus_dir / Path(*subdir.split("/"))).resolve()
        try:
            target.relative_to(root_resolved)
        except ValueError:
            errors.append(f"ESCAPE [{sec_name}] {subdir}")
            continue

        expected_entries = {
            validate_rel_posix_path(str(item["path"])): item
            for item in sec.get("files", [])
        }

        if not target.exists():
            if sec.get("present") or expected_entries:
                if expected_entries:
                    for rel_path in sorted(expected_entries):
                        errors.append(f"MISSING [{sec_name}] {subdir}/{rel_path}")
                else:
                    errors.append(f"MISSING_DIR [{sec_name}] {subdir}")
            continue

        if not target.is_dir():
            errors.append(f"NOT_A_DIR [{sec_name}] {subdir}")
            continue

        actual_paths: dict[str, Path] = {}
        for file_path in target.rglob("*"):
            if not file_path.is_file():
                continue
            if file_path.resolve() == manifest_resolved:
                continue
            rel = validate_rel_posix_path(file_path.relative_to(target).as_posix())
            actual_paths[rel] = file_path

        for rel_path in sorted(expected_entries):
            exp = expected_entries[rel_path]
            disk_path = actual_paths.get(rel_path)
            if disk_path is None:
                errors.append(f"MISSING [{sec_name}] {subdir}/{rel_path}")
                continue
            size, digest = sha256_file(disk_path)
            if size != exp.get("bytes") or digest != exp.get("sha256"):
                errors.append(
                    f"MISMATCH [{sec_name}] {subdir}/{rel_path} "
                    f"(expected sha256={exp.get('sha256')} bytes={exp.get('bytes')}, "
                    f"actual sha256={digest} bytes={size})"
                )

        for rel_path in sorted(actual_paths):
            if rel_path not in expected_entries:
                errors.append(f"UNEXPECTED [{sec_name}] {subdir}/{rel_path}")

    return len(errors) == 0, errors, manifest


def _build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="corpus_index.py",
        description="Generate or verify corpus.manifest.json for P&ID phase-1 corpora.",
    )
    sub = parser.add_subparsers(dest="command", required=True)

    p_index = sub.add_parser("index", help="Scan corpus and write corpus.manifest.json")
    p_index.add_argument("--dir", default=None, help=f"Corpus root (defaults to ${CORPUS_ENV})")
    p_index.add_argument("--manifest", default=None, help="Manifest path (defaults to <dir>/corpus.manifest.json)")
    p_index.add_argument(
        "--map",
        action="append",
        default=[],
        metavar="KEY=REL_SUBDIR",
        help="Map logical section KEY to relative subdirectory REL_SUBDIR",
    )
    p_index.add_argument(
        "--reset-layout",
        action="store_true",
        help="Ignore existing subdir mappings in prior manifest",
    )

    p_verify = sub.add_parser("verify", help="Verify corpus files against corpus.manifest.json")
    p_verify.add_argument("--dir", default=None, help=f"Corpus root (defaults to ${CORPUS_ENV})")
    p_verify.add_argument("--manifest", default=None, help="Manifest path (defaults to <dir>/corpus.manifest.json)")

    return parser


def _resolve_corpus_dir(arg_dir: str | None) -> Path | None:
    if arg_dir is not None:
        raw = arg_dir.strip()
        if not raw:
            raise ValueError("--dir cannot be empty")
        return Path(raw)
    env_val = os.environ.get(CORPUS_ENV, "").strip()
    if not env_val:
        return None
    return Path(env_val)


def main(argv: Sequence[str] | None = None) -> int:
    _ensure_utf8_stdio()
    parser = _build_parser()
    try:
        args = parser.parse_args(list(argv) if argv is not None else None)
    except SystemExit as exc:
        code = exc.code if isinstance(exc.code, int) else 2
        return 0 if code == 0 else 2

    try:
        corpus_dir = _resolve_corpus_dir(args.dir)
    except ValueError as err:
        print(f"error: {err}", file=sys.stderr)
        return 2

    if corpus_dir is None:
        print(f"未设置语料目录（{CORPUS_ENV}），跳过语料清单操作。")
        return 0

    if not corpus_dir.is_dir():
        print(f"error: corpus directory does not exist: {corpus_dir}", file=sys.stderr)
        return 2

    manifest_path = Path(args.manifest) if args.manifest else (corpus_dir / MANIFEST_FILENAME)

    if args.command == "index":
        try:
            overrides = _parse_mappings(args.map)
            manifest = build_manifest(
                corpus_dir,
                manifest_path=manifest_path,
                overrides=overrides,
                reset_layout=args.reset_layout,
            )
            manifest_path.parent.mkdir(parents=True, exist_ok=True)
            manifest_path.write_text(
                json.dumps(manifest, ensure_ascii=False, indent=2) + "\n",
                encoding="utf-8",
            )
        except (OSError, ValueError) as err:
            print(f"error: {err}", file=sys.stderr)
            return 2

        present_secs = sum(1 for sec in manifest["sections"].values() if sec["present"])
        print(
            f"indexed {manifest['total_files']} file(s) across {present_secs} present section(s) "
            f"(manifest_sha256={manifest['manifest_sha256'][:16]}...)"
        )
        return 0

    if args.command == "verify":
        if not manifest_path.is_file():
            print(f"error: manifest file not found: {manifest_path}", file=sys.stderr)
            return 2
        try:
            ok, errors, manifest = verify_manifest(corpus_dir, manifest_path)
        except (OSError, ValueError, json.JSONDecodeError) as err:
            print(f"error: invalid manifest: {err}", file=sys.stderr)
            return 2

        if not ok:
            for line in errors:
                print(line, file=sys.stderr)
            print(
                f"verify FAILED: {len(errors)} problem(s) in corpus manifest "
                f"(manifest_sha256={manifest.get('manifest_sha256', '')[:16]}...)",
                file=sys.stderr,
            )
            return 1

        print(
            f"verify OK: {manifest['total_files']} file(s) matched "
            f"(manifest_sha256={manifest['manifest_sha256'][:16]}...)"
        )
        return 0

    return 2


if __name__ == "__main__":
    sys.exit(main())
