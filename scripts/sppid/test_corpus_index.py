"""Tests for scripts/sppid/corpus_index.py (L0-2 / #107)."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))

import corpus_index  # noqa: E402

SCRIPT = Path(__file__).resolve().parent / "corpus_index.py"


def put_bytes(path: Path, data: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)


def sha256_hex(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def test_unset_env_exits_zero_and_prints_notice(monkeypatch, capsys):
    monkeypatch.delenv("OCS_PID_CORPUS_DIR", raising=False)

    rc_index = corpus_index.main(["index"])
    out_index = capsys.readouterr()
    assert rc_index == 0
    assert "未设置语料目录" in (out_index.out + out_index.err)

    rc_verify = corpus_index.main(["verify"])
    out_verify = capsys.readouterr()
    assert rc_verify == 0
    assert "未设置语料目录" in (out_verify.out + out_verify.err)


def test_subprocess_unset_env_exits_zero():
    env = dict(os.environ)
    env.pop("OCS_PID_CORPUS_DIR", None)
    proc = subprocess.run(
        [sys.executable, str(SCRIPT), "verify"],
        env=env,
        capture_output=True,
        text=True,
        encoding="utf-8",
        check=False,
    )
    assert proc.returncode == 0
    assert "未设置语料目录" in (proc.stdout + proc.stderr)


def test_default_layout_index_and_verify(tmp_path: Path, capsys):
    corpus = tmp_path / "corpus"
    pid_bytes = b"CFB-FAKE-PID-001"
    data_bytes = b"\xef\xbb\xbf<?xml version =\"1.0\" encoding=\"UTF-8\"?>\r\n<Container />\r\n"
    meta_bytes = b"\xef\xbb\xbf<?xml version =\"1.0\" encoding=\"UTF-8\"?>\r\n<Container Scope=\"Data\" />\r\n"
    sym_bytes = b"SYM-VALVE-GATE"

    put_bytes(corpus / "export-set" / "batch01" / "DWG01.pid", pid_bytes)
    put_bytes(corpus / "export-set" / "batch01" / "DWG01_Data.xml", data_bytes)
    put_bytes(corpus / "export-set" / "batch01" / "DWG01_Meta.xml", meta_bytes)
    put_bytes(corpus / "symbols" / "Piping" / "gate.sym", sym_bytes)

    assert corpus_index.main(["index", "--dir", str(corpus)]) == 0
    capsys.readouterr()

    manifest_path = corpus / "corpus.manifest.json"
    assert manifest_path.is_file()
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))

    assert manifest["schema_version"] == 1
    assert len(manifest["manifest_sha256"]) == 64
    export_sec = manifest["sections"]["EXPORT_SET"]
    assert export_sec["subdir"] == "export-set"
    assert export_sec["present"] is True
    assert export_sec["file_count"] == 3
    assert [f["path"] for f in export_sec["files"]] == [
        "batch01/DWG01.pid",
        "batch01/DWG01_Data.xml",
        "batch01/DWG01_Meta.xml",
    ]
    assert export_sec["files"][0]["sha256"] == sha256_hex(pid_bytes)
    assert export_sec["files"][0]["bytes"] == len(pid_bytes)

    symbols_sec = manifest["sections"]["SYMBOLS"]
    assert symbols_sec["present"] is True
    assert symbols_sec["file_count"] == 1
    assert symbols_sec["files"][0]["path"] == "Piping/gate.sym"

    seeds_sec = manifest["sections"]["SEEDS"]
    assert seeds_sec["present"] is False
    assert seeds_sec["file_count"] == 0

    assert corpus_index.main(["verify", "--dir", str(corpus)]) == 0


def test_tampered_single_byte_fails_verify_and_names_file(tmp_path: Path, capsys):
    corpus = tmp_path / "corpus"
    target = corpus / "export-set" / "01" / "P101_Data.xml"
    put_bytes(target, b"original-xml-payload-bytes")
    put_bytes(corpus / "symbols" / "a.sym", b"sym-bytes")

    assert corpus_index.main(["index", "--dir", str(corpus)]) == 0
    capsys.readouterr()

    raw = bytearray(target.read_bytes())
    raw[0] ^= 0x01
    target.write_bytes(bytes(raw))

    rc = corpus_index.main(["verify", "--dir", str(corpus)])
    captured = capsys.readouterr()
    combined = captured.out + captured.err
    assert rc == 1
    assert "01/P101_Data.xml" in combined


def test_missing_and_unexpected_files_fail_verify(tmp_path: Path, capsys):
    corpus = tmp_path / "corpus"
    f1 = corpus / "symbols" / "v1.sym"
    f2 = corpus / "symbols" / "v2.sym"
    put_bytes(f1, b"one")
    put_bytes(f2, b"two")

    assert corpus_index.main(["index", "--dir", str(corpus)]) == 0
    capsys.readouterr()

    # Extra unindexed file in tracked section must be caught.
    extra = corpus / "symbols" / "untracked.sym"
    put_bytes(extra, b"extra")
    rc_extra = corpus_index.main(["verify", "--dir", str(corpus)])
    out_extra = capsys.readouterr()
    assert rc_extra == 1
    assert "untracked.sym" in (out_extra.out + out_extra.err)
    extra.unlink()

    # Missing file must be caught and named.
    f2.unlink()
    rc_missing = corpus_index.main(["verify", "--dir", str(corpus)])
    out_missing = capsys.readouterr()
    assert rc_missing == 1
    assert "v2.sym" in (out_missing.out + out_missing.err)


def test_custom_mapping_preserves_layout_and_has_zero_absolute_paths(tmp_path: Path, capsys):
    corpus = tmp_path / "plant_root"
    put_bytes(corpus / "publish" / "sppid_out" / "01" / "A01.pid", b"pid-a01")
    put_bytes(corpus / "publish" / "sppid_out" / "01" / "A01_Data.xml", b"data-a01")
    put_bytes(corpus / "publish" / "sppid_out" / "01" / "A01_Meta.xml", b"meta-a01")
    put_bytes(corpus / "Ref" / "Symbols" / "inst.sym", b"sym-inst")

    manifest_path = tmp_path / "evidence" / "corpus.manifest.json"
    before_entries = sorted(p.relative_to(corpus).as_posix() for p in corpus.rglob("*"))

    rc = corpus_index.main(
        [
            "index",
            "--dir",
            str(corpus),
            "--manifest",
            str(manifest_path),
            "--map",
            "EXPORT_SET=publish/sppid_out",
            "--map",
            "SYMBOLS=Ref/Symbols",
        ]
    )
    capsys.readouterr()
    assert rc == 0

    # Corpus directory stayed strictly read-only when --manifest is external.
    after_entries = sorted(p.relative_to(corpus).as_posix() for p in corpus.rglob("*"))
    assert after_entries == before_entries

    raw_text = manifest_path.read_text(encoding="utf-8")
    assert "\\" not in raw_text
    assert str(corpus) not in raw_text
    assert corpus.as_posix() not in raw_text

    manifest = json.loads(raw_text)
    for sec_name, sec in manifest["sections"].items():
        subdir = sec["subdir"]
        assert not subdir.startswith("/"), sec_name
        assert ":" not in subdir, sec_name
        assert ".." not in subdir.split("/"), sec_name
        for entry in sec["files"]:
            rel = entry["path"]
            assert not rel.startswith("/"), rel
            assert ":" not in rel, rel
            assert "\\" not in rel, rel

    # Re-indexing against the same manifest without --map preserves custom layout.
    assert (
        corpus_index.main(
            ["index", "--dir", str(corpus), "--manifest", str(manifest_path)]
        )
        == 0
    )
    manifest_again = json.loads(manifest_path.read_text(encoding="utf-8"))
    assert manifest_again["sections"]["EXPORT_SET"]["subdir"] == "publish/sppid_out"
    assert manifest_again["sections"]["SYMBOLS"]["subdir"] == "Ref/Symbols"
    assert manifest_again["manifest_sha256"] == manifest["manifest_sha256"]

    assert (
        corpus_index.main(
            ["verify", "--dir", str(corpus), "--manifest", str(manifest_path)]
        )
        == 0
    )


def test_rejects_absolute_or_escaping_mapping_paths(tmp_path: Path, capsys):
    corpus = tmp_path / "corpus"
    corpus.mkdir()

    rc_escape = corpus_index.main(
        ["index", "--dir", str(corpus), "--map", "EXPORT_SET=../outside"]
    )
    capsys.readouterr()
    assert rc_escape == 2

    rc_abs = corpus_index.main(
        ["index", "--dir", str(corpus), "--map", "EXPORT_SET=C:/outside"]
    )
    capsys.readouterr()
    assert rc_abs == 2


def test_env_corpus_dir_when_configured(tmp_path: Path, capsys):
    env_dir = os.environ.get("OCS_PID_CORPUS_DIR", "").strip()
    if not env_dir:
        rc = corpus_index.main(["verify"])
        out = capsys.readouterr()
        assert rc == 0
        assert "未设置语料目录" in (out.out + out.err)
        return

    corpus = Path(env_dir)
    default_manifest = corpus / "corpus.manifest.json"
    if default_manifest.is_file():
        assert corpus_index.main(["verify"]) == 0
    else:
        temp_manifest = tmp_path / "corpus.manifest.json"
        assert (
            corpus_index.main(["index", "--manifest", str(temp_manifest)]) == 0
        )
        assert (
            corpus_index.main(["verify", "--manifest", str(temp_manifest)]) == 0
        )
