"""Tests of export_compare.py: what it makes of the folders export-compare.ps1 leaves."""

import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))

from export_compare import main  # noqa: E402
from walk_summary import HOME  # noqa: E402

NOT_A_DRAWING = "the drawing could not be read: not a SmartPlant drawing"


def put(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    if isinstance(value, bytes):
        path.write_bytes(value)
    elif isinstance(value, str):
        path.write_text(value, encoding="utf-8")
    else:
        path.write_text(json.dumps(value, ensure_ascii=False, indent=2), encoding="utf-8")


def change(path, how):
    """Rewrite a JSON file through `how`, which changes the value in place."""
    value = json.loads(path.read_text(encoding="utf-8"))
    how(value)
    put(path, value)


def replace(path, old, new):
    path.write_bytes(path.read_bytes().replace(old, new))


def drawn(name, plant):
    """The files of one drawing, as an export writes them."""
    xml = '<?xml version ="1.0"?>\n<Container Plant="{}" DocName="{}">{}</Container>\n'
    return {
        f"{name}_Data.xml": xml.format(plant, name, "data").encode(),
        f"{name}_Meta.xml": xml.format(plant, name, "meta").encode(),
        f"{name}_Coverage.json": json.dumps({"drawing": name}).encode(),
    }


def export(
    tmp_path,
    name="small",
    *,
    package="small",
    form="folder",
    plant="P01",
    reference=None,
    broken=None,
    drawings=("001", "002", "003"),
    expect=None,
    source=None,
    seconds=12.5,
    limit=None,
    sql_server=False,
    gate=None,
    warnings=(),
    exit_code=0,
):
    """Lay out the folders of one export in which everything agrees; return its entry of runs.json."""
    folder = tmp_path / name
    source = source or f"D:/samples/{package}" + (".zip" if form == "zip" else "")
    files, rows = {}, []
    for at, drawing in enumerate(drawings, 1):
        made = {} if drawing == broken else drawn(drawing, plant)
        files.update(made)
        rows.append({"name": drawing, "zip_file": "PlantData~2~711.zip", "zip_entry": f"zcgc/A/{at:03}.pid", "files": sorted(made)})
    index = {"package": source, "pid_entries": len(drawings), "template_pid_entries": 0, "unmatched_pid_entries": [], "drawings": rows}
    files["_Index.json"] = json.dumps(index, ensure_ascii=False).encode()
    for side in ("export", "cli"):
        for file, data in files.items():
            put(folder / side / file, data)

    written, failed = len(drawings) - bool(broken), int(bool(broken))
    stderr = [f"warning: {line}" for line in warnings]
    gate_lines = []
    if gate:
        clean, dirty, revision, unpaired = gate
        gate_lines = [f"{drawing}: clean" for drawing in drawings[:clean]]
        stderr.append(
            f"Package gate against {reference}: {sum(gate)} drawings, {clean} clean, {dirty} with findings, "
            f"{revision} of another revision, {unpaired} without a reference; exit 1."
        )
    stderr.append(f"Published {written} drawings from {source} ({failed} failed, 0 without a DrawingNo). Wrote {folder / 'cli' / '_Index.json'}.")
    put(folder / "cli.stderr.txt", "\n".join(stderr) + "\n")
    put(folder / "cli.stdout.txt", "".join(line + "\n" for line in gate_lines))

    ended = [f"PIDBACKUPEXPORT: wrote {written} drawings to {folder / 'export'}"]
    if sql_server:
        ended.append("PIDBACKUPEXPORT: a SQL Server package: what its export writes has not been verified")
    outcome = {
        "written": written,
        "failed": failed,
        "pid_entries": len(drawings),
        "templates": 0,
        "unmatched": [],
        "without_drawing_no": 0,
        "unwritten": [{"name": broken, "notes": [NOT_A_DRAWING]}] if broken else [],
        "warnings": list(warnings),
        "gate": gate and dict(zip(("clean", "dirty", "revision_mismatch", "unpaired"), gate), lines=gate_lines),
        "sql_server": sql_server,
        "files": len(files),
    }
    kept = [
        {"what": "source", "folder": source, "before": {"files": 16, "bytes": 1000}, "after": {"files": 16, "bytes": 1000}, "changed": []},
        {"what": "restored", "folder": "D:/restores/x", "before": {"files": 7, "bytes": 500}, "after": {"files": 7, "bytes": 500}, "changed": []},
    ]
    started = f"PIDBACKUPEXPORT: exporting {source} to {folder / 'export'} (plant {plant}) ..."
    report = {
        "source": source,
        "plant": plant,
        "reference": reference,
        "debug_assertions": False,
        "export": {
            "started": [started],
            "status": [f"PIDBACKUPEXPORT: exporting to {folder / 'export'} (plant {plant}): 0 s so far"],
            "ended": ended,
            "seconds": seconds,
            "outcome": outcome,
            "error": None,
        },
        "kept": kept,
        "unmet": [],
    }
    if broken:
        entry = f"zcgc/A/{drawings.index(broken) + 1:03}.pid"
        report["broken"] = {"copied_from": source, "archive": "PlantData~2~711.zip", "entry": entry, "drawing": broken}
        report["source"] = str(folder / "package")
    put(folder / "report.json", report)
    return {
        "name": name,
        "package": package,
        "form": form,
        "source": source,
        "plant": plant,
        "reference": reference,
        "broken": bool(broken),
        "expect": expect or f"files={len(files)},written={written}",
        "folder": str(folder),
        "exit_code": exit_code,
        "seconds": seconds + 3,
        "limit_seconds": limit,
        "note": "",
        "tool": {
            "folder": str(folder / "cli"),
            "stdout": str(folder / "cli.stdout.txt"),
            "stderr": str(folder / "cli.stderr.txt"),
            "exit_code": 1 if broken or gate else 0,
            "seconds": 2.0,
        },
    }


def summed(tmp_path, capsys, *runs, partial=False, out=None):
    """Run the tool on a runs.json of `runs`: (its exit code, what it printed)."""
    path = tmp_path / "runs.json"
    put(path, {"partial": partial, "build": "Finished `test` profile [optimized]", "tools": "D:/tools/96ee00c", "runs": list(runs)})
    code = main([str(path)] + (["--out", str(out)] if out else []))
    return code, capsys.readouterr().out


def test_exports_that_agree_with_the_tool_pass(tmp_path, capsys):
    code, out = summed(tmp_path, capsys, export(tmp_path, "small"), export(tmp_path, "small-zip", form="zip"))
    assert code == 0, out
    row = (
        "| `small` | 小包（目录） | `P01`（6 / 6） | 3 | 0 | 3 / 0 / 0 | 0 | 10 | 相同 "
        "| 10 / 10（`_Data`、`_Meta` 6 / 6；`_Coverage`、`_Index` 4 / 4） "
        "| 不变（包 16 个文件；还原目录 7 个文件） | 12.5 s | `files=10,written=3` | 过 |"
    )
    assert row in out
    assert "- 小包：相同。6 个 `_Data.xml`、`_Meta.xml`，名单相同，逐字节相同（目录那次 10 个文件，zip 那次 10 个）。" in out
    assert "构建：Finished `test` profile [optimized]（cargo 的原话），debug_assertions 关。" in out
    assert "命令行上说的话（小包（目录））：" in out
    assert "结论：2 次整包导出全部通过，1 项目录和 zip 的比较相同。" in out


def test_a_number_off_the_expectation_is_marked_in_its_cell(tmp_path, capsys):
    code, out = summed(tmp_path, capsys, export(tmp_path, expect="files=11,written=3"))
    assert code == 1
    assert "| 10（不过） |" in out
    assert "不过：文件数 10，期望 11" in out
    assert "结论：1 处不过。" in out


def test_more_than_is_met_by_more_and_not_by_as_many(tmp_path, capsys):
    code, _ = summed(tmp_path, capsys, export(tmp_path, expect="files>9,written>2"))
    assert code == 0
    code, out = summed(tmp_path, capsys, export(tmp_path, expect="files>10"))
    assert code == 1
    assert "文件数 10，期望 >10" in out


def test_a_file_unlike_the_tools_is_named_with_the_first_byte_that_differs(tmp_path, capsys):
    run = export(tmp_path)
    target = tmp_path / "small" / "export" / "002_Data.xml"
    at = target.read_bytes().index(b">data<") + 3
    replace(target, b">data<", b">dbta<")
    code, out = summed(tmp_path, capsys, run)
    assert code == 1
    assert "和 pid_publish_xml 字节不同的文件 1 个（`002_Data.xml`）" in out
    size = target.stat().st_size
    assert f"- 小包（目录） `002_Data.xml`：本程序 {size} 字节，`pid_publish_xml` {size} 字节，第 {at} 个字节起不同。" in out
    assert "| 9 / 10（`_Data`、`_Meta` 5 / 6；`_Coverage`、`_Index` 4 / 4）（不过） |" in out


def test_a_file_on_one_side_only_is_named(tmp_path, capsys):
    run = export(tmp_path)
    (tmp_path / "small" / "cli" / "003_Meta.xml").unlink()
    code, out = summed(tmp_path, capsys, run)
    assert code == 1
    assert "只有本程序写出的文件 `003_Meta.xml`" in out


def test_numbers_unlike_the_tools_are_named(tmp_path, capsys):
    run = export(tmp_path)
    replace(tmp_path / "small" / "cli.stderr.txt", b"Published 3 drawings", b"Published 2 drawings")
    code, out = summed(tmp_path, capsys, run)
    assert code == 1
    assert "和 pid_publish_xml 报的不同（本程序 / 它）：写出 3 / 2" in out
    assert "| 不同：写出 3 / 2（不过） |" in out


def test_warnings_unlike_the_tools_are_named(tmp_path, capsys):
    run = export(tmp_path, warnings=("2 drawings share a DrawingNo",))
    replace(tmp_path / "small" / "cli.stderr.txt", b"warning: 2 drawings", b"warning: 3 drawings")
    code, out = summed(tmp_path, capsys, run)
    assert code == 1
    assert "提醒 1 句 / 1 句，内容不同" in out


def test_a_tool_that_said_nothing_of_what_it_wrote_does_not_pass(tmp_path, capsys):
    run = export(tmp_path)
    put(tmp_path / "small" / "cli.stderr.txt", "error: the backup cannot be read\n")
    run["tool"]["exit_code"] = 101
    code, out = summed(tmp_path, capsys, run)
    assert code == 1
    assert "pid_publish_xml 失败（退出码 101）" in out
    assert "pid_publish_xml 没有报写出几张" in out


def test_an_export_without_the_tool_does_not_pass(tmp_path, capsys):
    run = export(tmp_path)
    run["tool"] = None
    code, out = summed(tmp_path, capsys, run)
    assert code == 1
    assert "没有跑 pid_publish_xml" in out


def test_a_file_without_the_plant_name_does_not_pass(tmp_path, capsys):
    run = export(tmp_path, plant="SQPlant")
    for side in ("export", "cli"):
        replace(tmp_path / "small" / side / "001_Data.xml", b'Plant="SQPlant"', b'Plant="P01"')
    code, out = summed(tmp_path, capsys, run)
    assert code == 1
    assert "| `SQPlant`（5 / 6）（不过） |" in out
    assert "没有写工厂名 `SQPlant` 的文件 `001_Data.xml`" in out


def test_a_stray_file_in_the_export_does_not_pass(tmp_path, capsys):
    run = export(tmp_path)
    put(tmp_path / "small" / "export" / "stray.txt", "left over")
    code, out = summed(tmp_path, capsys, run)
    assert code == 1
    assert "| 11（另有 1 个不是导出写的）（不过） |" in out
    assert "| 10 / 11（`_Data`、`_Meta` 6 / 6；`_Coverage`、`_Index` 4 / 4；别的文件 0 / 1）（不过） |" in out
    assert "导出目录里有不是导出写的文件：`stray.txt`" in out
    assert "导出说写了 10 个文件，目录里有 11 个" in out


def test_a_restore_that_changed_does_not_pass(tmp_path, capsys):
    run = export(tmp_path)

    def grown(report):
        report["kept"][1]["after"]["files"] = 8
        report["kept"][1]["changed"] = ["zcgc/A/x.pid"]

    change(tmp_path / "small" / "report.json", grown)
    code, out = summed(tmp_path, capsys, run)
    assert code == 1
    assert "还原目录变了（文件 7 → 8：`zcgc/A/x.pid`）" in out
    assert "| 变了（不过） |" in out


def test_an_export_over_its_time_does_not_pass(tmp_path, capsys):
    code, out = summed(tmp_path, capsys, export(tmp_path, "plant", package="plant", seconds=75.0, limit=60))
    assert code == 1
    assert "| 75.0 s（不过） |" in out
    assert "导出用了 75 秒，超过 60 秒" in out
    assert "全厂包（目录）的导出不能超过 60 秒。" in out


def test_a_sql_server_package_says_it_was_not_verified(tmp_path, capsys):
    run = export(tmp_path, "sqlserver", package="sqlserver", sql_server=True)
    code, out = summed(tmp_path, capsys, run)
    assert code == 0, out
    assert "| SQL Server 包（目录），没有验收过 |" in out
    assert "本程序的结果里有「没有验收过」那一句。" in out

    change(tmp_path / "sqlserver" / "report.json", lambda report: report["export"]["ended"].pop())
    code, out = summed(tmp_path, capsys, run)
    assert code == 1
    assert "SQL Server 包的结果里没有说「没有验收过」" in out


def test_the_broken_drawing_is_named_and_the_others_are_written(tmp_path, capsys):
    run = export(tmp_path, "small-broken", broken="002", expect="files=7,written=2,unwritten=1,failed>0")
    code, out = summed(tmp_path, capsys, run)
    assert code == 0, out
    assert (
        "- 小包（目录，换坏一张）：把 `PlantData~2~711.zip` 里的 `zcgc/A/002.pid` 换成不是图的内容。"
        f"它是 `002`，在没写出的名单里，原因：{NOT_A_DRAWING}。其余 2 张写出 2 张，失败计数 1。"
    ) in out
    assert f"- 小包（目录，换坏一张） `002`：{NOT_A_DRAWING}" in out


def test_a_broken_drawing_without_a_reason_does_not_pass(tmp_path, capsys):
    run = export(tmp_path, "small-broken", broken="002", expect="files=7,written=2,unwritten=1,failed>0")
    change(tmp_path / "small-broken" / "report.json", lambda report: report["export"]["outcome"]["unwritten"].clear())
    code, out = summed(tmp_path, capsys, run)
    assert code == 1
    assert "没写出的名单里没有换坏的那张 `002`，或者没有原因" in out
    assert "没写出的图 0，期望 1" in out


def test_a_broken_copy_whose_other_drawings_are_not_written_does_not_pass(tmp_path, capsys):
    run = export(tmp_path, "small-broken", broken="002", expect="written=2")

    def emptied(index):
        index["drawings"][2]["files"] = []

    for side in ("export", "cli"):
        change(tmp_path / "small-broken" / side / "_Index.json", emptied)
    code, out = summed(tmp_path, capsys, run)
    assert code == 1
    assert "没换坏的图也没写出：`003`" in out


def test_the_gate_of_both_sides_is_shown_and_held_alike(tmp_path, capsys):
    run = export(tmp_path, "plant-ref", package="plant", reference="D:/ref", gate=(2, 0, 1, 0))
    code, out = summed(tmp_path, capsys, run)
    assert code == 0, out
    assert "- 全厂包（目录，带参考目录），参考目录 `D:/ref`：本程序 2 张一致，0 张有差异，1 张的参考是另一版，0 张没有参考。" in out
    assert "  `pid_publish_xml --diff-against`：2 张一致，0 张有差异，1 张的参考是另一版，0 张没有参考；逐张的比对 2 行，逐行相同。" in out

    replace(tmp_path / "plant-ref" / "cli.stderr.txt", b"2 clean, 0 with findings", b"1 clean, 1 with findings")
    code, out = summed(tmp_path, capsys, run)
    assert code == 1
    assert "比对结论 2/0/1/0 / 1/1/1/0" in out


def test_a_gate_with_nothing_compared_is_read_too(tmp_path, capsys):
    run = export(tmp_path, "plant-ref", package="plant", reference="D:/ref", gate=(0, 0, 1, 2), expect="gate=0/0/1/2")
    put(
        tmp_path / "plant-ref" / "cli.stderr.txt",
        "Package gate against D:/ref: none of the 3 written drawings has a comparable reference there "
        "(1 of another revision, 2 without a reference); nothing was compared; exit 1.\n"
        "Published 3 drawings from D:/samples/plant (0 failed, 0 without a DrawingNo). Wrote x.\n",
    )
    code, out = summed(tmp_path, capsys, run)
    assert code == 0, out


def test_folder_and_zip_that_wrote_other_bytes_do_not_agree(tmp_path, capsys):
    runs = [export(tmp_path, "small"), export(tmp_path, "small-zip", form="zip")]
    for side in ("export", "cli"):
        replace(tmp_path / "small-zip" / side / "001_Data.xml", b">data<", b">dbta<")
    code, out = summed(tmp_path, capsys, *runs)
    assert code == 1
    assert "- 小包：不同。字节不同的 `001_Data.xml`。" in out
    assert "- 目录和 zip 的比较：小包" in out


def test_another_plant_name_differs_in_the_plant_name_only(tmp_path, capsys):
    runs = [export(tmp_path, "plant", package="plant"), export(tmp_path, "plant-sqplant", package="plant", plant="SQPlant")]
    code, out = summed(tmp_path, capsys, *runs)
    assert code == 0, out
    assert "- 全厂包：`P01` 和 `SQPlant` 两次导出，6 个 `_Data.xml`、`_Meta.xml` 里，6 个只有工厂名不同。" in out

    for side in ("export", "cli"):
        replace(tmp_path / "plant-sqplant" / side / "001_Meta.xml", b">meta<", b">mfta<")
    code, out = summed(tmp_path, capsys, *runs)
    assert code == 0, out
    assert "6 个 `_Data.xml`、`_Meta.xml` 里，5 个只有工厂名不同，1 个换掉工厂名以后还不同。" in out


def test_an_export_that_was_not_run_does_not_pass(tmp_path, capsys):
    run = export(tmp_path)
    run.update(exit_code=None, note="-Only 没有选它")
    code, out = summed(tmp_path, capsys, run, partial=True)
    assert code == 1
    assert "这次只跑了一部分，不是完整的验收。" in out
    assert "没有导出（-Only 没有选它）" in out


def test_a_failed_test_shows_what_it_says(tmp_path, capsys):
    run = export(tmp_path, exit_code=101)
    change(tmp_path / "small" / "report.json", lambda report: report["unmet"].append("the export changed the restored folder"))
    code, out = summed(tmp_path, capsys, run)
    assert code == 1
    assert "导出的测试失败（退出码 101）；测试说：the export changed the restored folder" in out


def test_a_report_of_another_backup_does_not_pass(tmp_path, capsys):
    run = export(tmp_path)
    run["source"] = "D:/samples/other"
    code, out = summed(tmp_path, capsys, run)
    assert code == 1
    assert "报告的来源是 `D:/samples/small`，这次导的是 `D:/samples/other`" in out


def test_an_export_that_came_to_an_error_does_not_pass(tmp_path, capsys):
    run = export(tmp_path, exit_code=101)

    def failed(report):
        report["export"]["outcome"] = None
        report["export"]["error"] = {"code": "Io", "text": "the folder cannot be written"}

    change(tmp_path / "small" / "report.json", failed)
    code, out = summed(tmp_path, capsys, run)
    assert code == 1
    assert "导出没有成功（Io：the folder cannot be written）" in out


def test_the_time_of_all_runs_is_summed(tmp_path, capsys):
    run = export(tmp_path)
    code, out = summed(tmp_path, capsys, run)
    assert code == 0, out
    assert "1 次测试共用 16 秒；`pid_publish_xml` 1 次共用 2 秒。" in out

    run["tool"]["seconds"] = 0.3
    code, out = summed(tmp_path, capsys, run)
    assert code == 0, out
    assert "`pid_publish_xml` 1 次共用不到 1 秒。" in out


def test_nothing_run_does_not_pass(tmp_path, capsys):
    code, out = summed(tmp_path, capsys)
    assert code == 1
    assert "结论：什么也没有跑。" in out


def test_runs_that_cannot_be_used_stop_with_2(tmp_path, capsys):
    for broken in (
        {"expect": "fils=10"},
        {"expect": "gate=1/2"},
        {"expect": "gate>1/2/3/4"},
        {"expect": "files=10,files=11"},
        {"form": "tar"},
        {"folder": ""},
        {"exit_code": "0"},
    ):
        run = export(tmp_path)
        run.update(broken)
        code, _ = summed(tmp_path, capsys, run)
        assert code == 2, broken
    code, _ = summed(tmp_path, capsys, export(tmp_path), export(tmp_path))
    assert code == 2
    assert main([str(tmp_path / "nowhere.json")]) == 2
    assert "export_compare: " in capsys.readouterr().err


def test_a_home_folder_is_hidden_and_the_file_has_plain_newlines(tmp_path, capsys):
    source = "C:\\Users\\someone\\samples\\small"
    out_file = tmp_path / "summary.md"
    code, out = summed(tmp_path, capsys, export(tmp_path, source=source), out=out_file)
    assert code == 0, out
    assert "someone" not in out and HOME.search(out) is None
    assert "%USERPROFILE%" in out
    written = out_file.read_bytes()
    assert b"\r\n" not in written
    assert written.decode("utf-8") == out
