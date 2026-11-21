"""Tests for scripts/sppid/corpus_correspondence.py (L0-3 / #120)."""

from __future__ import annotations

import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))

import corpus_correspondence  # noqa: E402
import corpus_index  # noqa: E402


def put(path: Path, data: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)


def test_unset_env_exits_zero(monkeypatch, capsys):
    monkeypatch.delenv("OCS_PID_CORPUS_DIR", raising=False)
    rc = corpus_correspondence.main([])
    captured = capsys.readouterr()
    assert rc == 0
    assert "未设置语料目录" in (captured.out + captured.err)


def test_exact_fuzzy_and_empty_dxf_correspondence(tmp_path: Path, capsys):
    corpus = tmp_path / "corpus"
    # 1. Exact full match
    put(corpus / "export-set" / "01" / "UnitA_Separator.pid", b"pid1")
    put(corpus / "export-set" / "01" / "UnitA_Separator_Data.xml", b"data1")
    put(corpus / "export-set" / "01" / "UnitA_Separator_Meta.xml", b"meta1")
    put(corpus / "export-dxf" / "UnitA_Separator.dxf", b"dxf1-nonempty")
    put(corpus / "export-pdf" / "UnitA_Separator.pdf", b"pdf1-nonempty")

    # 2. Fuzzy 1-char rename (集 vs 及)
    put(corpus / "export-set" / "01" / "站场 空冷器A集预冷分离器A流程图.pid", b"pid2")
    put(corpus / "export-set" / "01" / "站场 空冷器A集预冷分离器A流程图_Data.xml", b"data2")
    put(corpus / "export-set" / "01" / "站场 空冷器A集预冷分离器A流程图_Meta.xml", b"meta2")
    put(corpus / "export-dxf" / "站场 空冷器A及预冷分离器A流程图.dxf", b"dxf2-nonempty")
    put(corpus / "export-pdf" / "站场 空冷器A及预冷分离器A流程图.pdf", b"pdf2-nonempty")

    # 3. 0-byte empty DXF
    put(corpus / "export-set" / "02" / "UnitB_Retrofit.pid", b"pid3")
    put(corpus / "export-set" / "02" / "UnitB_Retrofit_Data.xml", b"data3")
    put(corpus / "export-set" / "02" / "UnitB_Retrofit_Meta.xml", b"meta3")
    put(corpus / "export-dxf" / "UnitB_Retrofit.dxf", b"")
    put(corpus / "export-pdf" / "UnitB_Retrofit.pdf", b"pdf3-nonempty")

    manifest_path = tmp_path / "out" / "corpus.manifest.json"
    out_dir = tmp_path / "out"
    assert corpus_index.main(["index", "--dir", str(corpus), "--manifest", str(manifest_path)]) == 0
    capsys.readouterr()

    rc = corpus_correspondence.main(
        ["--dir", str(corpus), "--manifest", str(manifest_path), "--out-dir", str(out_dir)]
    )
    capsys.readouterr()
    assert rc == 0

    raw = manifest_path.read_text(encoding="utf-8")
    assert "\\" not in raw
    assert str(corpus) not in raw
    manifest = json.loads(raw)

    corr = manifest["correspondence"]
    assert corr["summary"]["total_pid"] == 3
    assert corr["summary"]["complete_xml_triples"] == 3
    assert corr["summary"]["dxf_exact"] == 2
    assert corr["summary"]["dxf_fuzzy"] == 1
    assert corr["summary"]["dxf_empty_zero_bytes"] == 1
    assert corr["summary"]["usable_dxf_for_recognition"] == 2

    assert (out_dir / "correspondence.csv").is_file()
    assert (out_dir / "correspondence-summary.md").is_file()

    # Corpus verify must still pass after adding correspondence!
    assert corpus_index.main(["verify", "--dir", str(corpus), "--manifest", str(manifest_path)]) == 0


def test_missing_xml_fails_strict_and_names_item(tmp_path: Path, capsys):
    corpus = tmp_path / "corpus"
    put(corpus / "export-set" / "01" / "Broken.pid", b"pid")
    put(corpus / "export-set" / "01" / "Broken_Data.xml", b"data")
    # Missing Broken_Meta.xml!

    manifest_path = tmp_path / "out" / "corpus.manifest.json"
    assert corpus_index.main(["index", "--dir", str(corpus), "--manifest", str(manifest_path)]) == 0
    capsys.readouterr()

    rc = corpus_correspondence.main(
        ["--dir", str(corpus), "--manifest", str(manifest_path), "--strict"]
    )
    out = capsys.readouterr()
    assert rc == 1
    assert "Broken" in (out.out + out.err)
