#!/usr/bin/env python3
"""Sum up the exports that export-compare.ps1 ran on the sample backups.

export-compare.ps1 has the program export every sample backup through the
command PIDBACKUPEXPORT (the test
`a_real_backup_is_exported_through_the_command_and_stays_as_it_was`; for the
copy with one drawing made into none,
`a_real_backup_with_a_broken_drawing_names_it_and_writes_the_others`), has
`pid_publish_xml` export the very package the program exported, under the
same plant name and against the same reference folder, and leaves a
`runs.json` naming what it ran and the folder each step wrote to. This tool
reads those folders and prints, as Markdown ready for readings.md:

- one row per export: the plant name and whether every `_Data.xml` and
  `_Meta.xml` states it, the drawings written, the failure count, the `.pid`
  files and what became of them, the drawings without a `DrawingNo`, the
  files, whether `pid_publish_xml` says the same numbers and writes the same
  bytes, whether the backup and its restore stayed as they were, the time,
  the expectation, and whether the export passes;
- for every backup exported both from its folder and from its zip: whether
  the two exports wrote the same `_Data.xml` and `_Meta.xml`;
- for every export against a reference folder: the comparison of the program
  beside the one of `pid_publish_xml`;
- for the copy with a broken drawing: which drawing, the reason the program
  gives, and what became of the others;
- for an export under a plant name other than the default beside one under
  the default: whether their files differ in the plant name and in nothing
  else;
- what the command line said of the first export, from the line that starts
  it to its summary;
- the files that differ from the ones of `pid_publish_xml`, and the drawings
  that were not written, each with the reason.

    python scripts/sppid/export_compare.py RUNS_JSON [--out FILE]

An export passes when its test passed, its report is the report of that very
backup under that plant name, the export came to an outcome, the numbers
meet the expectation, its folder holds no file but the `_Data.xml`,
`_Meta.xml`, `_Coverage.json` of drawings and the `_Index.json`, every
`_Data.xml` and `_Meta.xml` states the plant name, `pid_publish_xml` ran
and says the same numbers, warnings and comparison, both folders hold the
same files with the same bytes, the backup and its restore have the same
files, of the same lengths and times, as before, the export stayed within its
time limit when it has one, the summary of a SQL Server package says that
what it writes has not been verified, and, for the copy with a broken
drawing, that drawing is named with a reason, lists no file in the index,
and every other drawing is written.

The verdicts are made here: the files of both folders are read and compared,
the output of `pid_publish_xml` is read for its numbers. What the report says
of itself is shown when the test failed and is not taken for the verdict.
Two things are taken from the report: what the program's export came to, and
the files of the backup before and after, which the test lists.

Exit code:

- 0: every export passes and every comparison agrees;
- 1: something does not pass, or nothing was run at all;
- 2: the arguments or runs.json cannot be used, or the file of --out cannot
  be written.

runs.json:

    {"partial": false, "build": "Finished `test` profile [optimized] ...",
     "tools": "D:/tools/96ee00c",
     "runs": [{"name": "small", "package": "small", "form": "folder",
               "source": "D:/samples/small", "plant": "P01",
               "reference": null, "broken": false,
               "expect": "files=22,written=7", "folder": "D:/out/small",
               "exit_code": 0, "seconds": 11.2, "limit_seconds": null,
               "note": "",
               "tool": {"folder": "D:/out/small/cli",
                        "stdout": "D:/out/small/cli.stdout.txt",
                        "stderr": "D:/out/small/cli.stderr.txt",
                        "exit_code": 0, "seconds": 2.5}}]}

`name` tells the exports apart; `package` is plant, small, sqlserver, or any
other name; `form` is folder or zip; `plant` is the plant name both sides
were given; `reference` the reference folder both were given, null for none;
`broken` whether the export is the one of the copy with a broken drawing;
`expect` is what is expected of the export, `key=N` or `key>N` joined by
commas, the keys being files, written, failed, pid, templates, unmatched,
shells, unwritten, and gate, whose value is four numbers: clean, with
findings, of another revision, without a reference, as in gate=7/0/4/42;
`folder` holds the test's report.json and its export, in the folder export;
`exit_code` is what cargo returned, null when the step was not run, and then
`note` says why; `limit_seconds` is how long the export may take, null for
any time. `tool` is null when `pid_publish_xml` was not run: its `folder`
holds what it wrote, `stdout` and `stderr` what it printed. `partial` is true
when only a part of the exports was run. `build` is what cargo said of the
test program and `tools` the folder of `pid_publish_xml`: the two are named
in the summary as they stand.

The Markdown is Chinese, as readings.md is. A home folder in it is written
%USERPROFILE%: the summary goes into a document that is committed.
"""

import argparse
from dataclasses import dataclass, field
import hashlib
import json
from pathlib import Path
import re
import sys

from compare_walks import _cell, _name, _reason
from walk_summary import FORMS, HOME, ManifestError, _package, _same_path, _span

REPORT = "report.json"
INDEX = "_Index.json"
# The folder of a run that holds the program's export.
EXPORT = "export"
DEFAULT_PLANT = "P01"
# The files of a drawing, by the end of their names.
KINDS = (("_Data.xml", "data"), ("_Meta.xml", "meta"), ("_Coverage.json", "coverage"))
# Of the drawings or files a reason is about, how many it names.
SHOWN = 8
# What the test watched for change, as the table names it.
KEPT = {"source": "包", "package": "包根目录", "restored": "还原目录"}
# The backup nothing was accepted on (SPEC-sppid-export §6, criterion 12).
UNACCEPTED = "sqlserver"
# What the summary of a SQL Server package says (SPEC-sppid-export §5.2).
UNVERIFIED = "has not been verified"
# What may be expected of an export, as the table names it.
EXPECTED = {
    "files": "文件数",
    "written": "写出",
    "failed": "失败计数",
    "pid": ".pid 数",
    "templates": "图框模板",
    "unmatched": "没对上的 .pid",
    "shells": "没有 DrawingNo",
    "unwritten": "没写出的图",
    "gate": "比对结论",
}
# What `pid_publish_xml` prints on stderr of a backup it published.
PUBLISHED = re.compile(r"Published (\d+) drawings from (.+) \((\d+) failed, (\d+) without a DrawingNo\)\. Wrote (.+)\.")
GATE = re.compile(
    r"Package gate against (.+): (\d+) drawings, (\d+) clean, (\d+) with findings, "
    r"(\d+) of another revision, (\d+) without a reference; exit (\d+)\."
)
GATE_NONE = re.compile(
    r"Package gate against (.+): none of the (\d+) written drawings has a comparable reference there "
    r"\((\d+) of another revision, (\d+) without a reference\); nothing was compared; exit (\d+)\."
)
WARNING = "warning: "


@dataclass
class Tool:
    """One run of `pid_publish_xml` on the package the program exported."""

    folder: Path
    stdout: Path
    stderr: Path
    exit_code: object = None
    seconds: object = None


@dataclass
class Told:
    """What one side says of an export."""

    written: object = None
    failed: object = None
    shells: object = None
    pid: object = None
    templates: object = None
    unmatched: object = None
    warnings: list = field(default_factory=list)
    # Clean, with findings, of another revision, without a reference; None
    # when there was no reference folder.
    gate: object = None
    gate_lines: list = field(default_factory=list)


@dataclass
class Export:
    """One export: what runs.json says of it, then what its folders hold."""

    name: str
    package: str
    form: str
    source: str
    plant: str
    reference: object
    broken: bool
    # Key -> ("=" or ">", the number, or the four numbers of the gate).
    expect: dict
    expect_text: str
    folder: Path
    exit_code: object = None
    seconds: object = None
    limit: object = None
    note: str = ""
    tool: object = None
    report: object = None
    ours: object = None
    theirs: object = None
    # The files of the program's export and of `pid_publish_xml`'s: name -> (length, SHA-256).
    files: object = None
    their_files: object = None
    # The cells of the table row that could be read.
    cells: dict = field(default_factory=dict)
    # The cells that do not pass.
    failing: set = field(default_factory=set)
    # The files that are not the same bytes on both sides: (name, ours, theirs, the first byte that differs).
    differing: list = field(default_factory=list)
    # The drawings the program did not write: (name, the reason it gives).
    unwritten: list = field(default_factory=list)
    debug: object = None
    # Why the export does not pass, as Markdown fit for a table cell.
    reasons: list = field(default_factory=list)


def _number(value):
    return isinstance(value, (int, float)) and not isinstance(value, bool)


def _whole(value):
    return isinstance(value, int) and not isinstance(value, bool)


def _load(path):
    """The JSON a file holds: (the value, ""), or (None, why it cannot be had)."""
    try:
        return json.loads(Path(path).read_text(encoding="utf-8-sig")), ""
    except FileNotFoundError:
        return None, "没有这个文件"
    except OSError as error:
        return None, f"读不了（{error.strerror or error}）"
    except UnicodeDecodeError:
        return None, "不是 UTF-8 文本"
    except json.JSONDecodeError as error:
        return None, f"不是 JSON（{error}）"


def _held(folder):
    """The files right in a folder: name -> (length, SHA-256 of the bytes)."""
    held = {}
    for entry in sorted(Path(folder).iterdir()):
        if entry.is_file():
            data = entry.read_bytes()
            held[entry.name] = (len(data), hashlib.sha256(data).hexdigest())
    return held


def _kind(name):
    """What a file of an export is: data, meta, coverage, index, or None for no file an export writes."""
    if name == INDEX:
        return "index"
    return next((kind for end, kind in KINDS if name.endswith(end) and len(name) > len(end)), None)


def _listing(names, unit="张"):
    """Names for a reason: the first few, then how many there are."""
    said = "、".join(_name(name) for name in names[:SHOWN])
    return said + (f" 等 {len(names)} {unit}" if len(names) > SHOWN else "")


def _spent(seconds):
    return f"共用 {_span(seconds)}" if seconds >= 0.5 else "共用不到 1 秒"


def _first_difference(one, other):
    """The first byte at which two files differ, counted from 1."""
    shared = min(len(one), len(other))
    return next((at + 1 for at in range(shared) if one[at] != other[at]), shared + 1)


def _label(run):
    """The export as a reason names it."""
    extra = []
    if run.plant != DEFAULT_PLANT:
        extra.append(f"工厂名 {run.plant}")
    if run.reference:
        extra.append("带参考目录")
    if run.broken:
        extra.append("换坏一张")
    return f"{_package(run.package)}（{'，'.join([FORMS[run.form], *extra])}）"


def _code(entry, which):
    """The exit code an entry of runs.json states: a whole number, or None."""
    code = entry.get("exit_code")
    if code is not None and not _whole(code):
        raise ManifestError(f"{which}的 exit_code 不是整数")
    return code


def _seconds(entry, key="seconds"):
    return entry[key] if _number(entry.get(key)) else None


def _expectation(text, which):
    """An expectation as runs.json spells it, `key=N` or `key>N` joined by commas: key -> (op, value)."""
    expect = {}
    for part in (part.strip() for part in text.split(",")):
        if not part:
            continue
        found = re.fullmatch(r"([a-z]+)\s*([=>])\s*(\d+(?:/\d+){3}|\d+)", part)
        if not found or found[1] not in EXPECTED:
            keys = "、".join(EXPECTED)
            raise ManifestError(f"{which}的期望值里的「{part}」不认识：写成 files=160、failed>0 这样，键是 {keys}")
        key, op, value = found.groups()
        if (key == "gate") != ("/" in value) or (key == "gate" and op != "="):
            raise ManifestError(f"{which}的期望值里的「{part}」不对：gate 写成 gate=7/0/4/42，别的键是一个数")
        if key in expect:
            raise ManifestError(f"{which}的期望值里 {key} 写了两次")
        expect[key] = (op, tuple(int(n) for n in value.split("/")) if key == "gate" else int(value))
    if not expect:
        raise ManifestError(f"{which}的期望值是空的")
    return expect


def load_runs(path):
    """What a runs.json names: (the exports, whether they are only a part, what it says of the build and the tools)."""
    manifest, why = _load(path)
    if manifest is None:
        raise ManifestError(f"{path}: {why or '是空的'}")
    if not isinstance(manifest, dict) or not isinstance(manifest.get("runs"), list):
        raise ManifestError(f"{path}: 没有 runs 一项")
    exports, met = [], {}
    for at, entry in enumerate(manifest["runs"], 1):
        which = f"{path}: 第 {at} 次导出"
        if not isinstance(entry, dict):
            raise ManifestError(f"{which}不是一个对象")
        for key in ("name", "package", "form", "source", "plant", "expect", "folder"):
            if not isinstance(entry.get(key), str) or not entry[key]:
                raise ManifestError(f"{which}缺 {key}")
        if entry["form"] not in FORMS:
            raise ManifestError(f"{which}的 form 是「{entry['form']}」，只能是 {' 或 '.join(FORMS)}")
        reference = entry.get("reference")
        if reference is not None and (not isinstance(reference, str) or not reference):
            raise ManifestError(f"{which}的 reference 不是一个路径")
        if not isinstance(entry.get("broken", False), bool):
            raise ManifestError(f"{which}的 broken 不是 true 或 false")
        if entry["name"] in met:
            raise ManifestError(f"{which}和第 {met[entry['name']]} 次同名（{entry['name']}）")
        met[entry["name"]] = at
        tool = entry.get("tool")
        if tool is not None:
            for key in ("folder", "stdout", "stderr"):
                if not isinstance(tool, dict) or not isinstance(tool.get(key), str) or not tool[key]:
                    raise ManifestError(f"{which}的 tool 缺 {key}")
            tool = Tool(
                Path(tool["folder"]),
                Path(tool["stdout"]),
                Path(tool["stderr"]),
                _code(tool, f"{which}的 tool "),
                _seconds(tool),
            )
        exports.append(
            Export(
                entry["name"],
                entry["package"],
                entry["form"],
                entry["source"],
                entry["plant"],
                reference,
                entry.get("broken", False),
                _expectation(entry["expect"], which),
                entry["expect"],
                Path(entry["folder"]),
                exit_code=_code(entry, which),
                seconds=_seconds(entry),
                limit=_seconds(entry, "limit_seconds"),
                note=str(entry.get("note") or ""),
                tool=tool,
            )
        )
    said = {key: str(manifest.get(key) or "") for key in ("build", "tools")}
    return exports, bool(manifest.get("partial")), said


# ── One export ──────────────────────────────────────────────────────────────────────────────────


def read_export(run):
    """Read what an export left in its folders, and note why it does not pass."""
    if run.exit_code is None:
        run.reasons.append(f"没有导出（{_cell(run.note)}）" if run.note else "没有导出")
        return
    if run.exit_code != 0:
        run.reasons.append(f"导出的测试失败（退出码 {run.exit_code}）")
    report, why = _load(run.folder / REPORT)
    if not isinstance(report, dict):
        run.reasons.append(f"测试的报告 {REPORT}：{_cell(why or '不是一个对象')}")
        return
    run.report = report
    broken = report.get("broken") if isinstance(report.get("broken"), dict) else None
    if run.broken != (broken is not None):
        run.reasons.append("报告不是换坏一张的那条测试写的" if run.broken else "报告是换坏一张的那条测试写的")
    source = str((broken or {}).get("copied_from") or report.get("source") or "")
    if not _same_path(source, run.source):
        run.reasons.append(f"报告的来源是 `{_cell(source)}`，这次导的是 `{_cell(run.source)}`")
    if str(report.get("plant") or "") != run.plant:
        run.reasons.append(f"报告的工厂名是「{_cell(str(report.get('plant')))}」，这次给的是「{_cell(run.plant)}」")
    if not _same_path(str(report.get("reference") or ""), run.reference or ""):
        run.reasons.append("报告的参考目录不是这次给的")
    unmet = report.get("unmet")
    if run.exit_code != 0 and isinstance(unmet, list) and unmet:
        said = "；".join(_reason(str(what)) for what in unmet[:3])
        run.reasons.append(f"测试说：{said}" + (f" 等 {len(unmet)} 条" if len(unmet) > 3 else ""))
    if isinstance(report.get("debug_assertions"), bool):
        run.debug = report["debug_assertions"]
    export = report.get("export") if isinstance(report.get("export"), dict) else {}
    outcome = export.get("outcome")
    if not isinstance(outcome, dict):
        error = export.get("error") if isinstance(export.get("error"), dict) else {}
        told = "：".join(str(error[key]) for key in ("code", "text") if error.get(key))
        run.reasons.append(f"导出没有成功（{_cell(told)}）" if told else "报告里没有导出的结果")
        return
    run.ours = _ours(outcome)
    for note in outcome.get("unwritten") or []:
        if isinstance(note, dict):
            notes = note.get("notes") if isinstance(note.get("notes"), list) else []
            run.unwritten.append((str(note.get("name")), "；".join(str(said) for said in notes)))
    try:
        run.files = _held(run.folder / EXPORT)
    except OSError as error:
        run.reasons.append(_cell(f"导出目录读不了（{error.strerror or error}）"))
        return
    _counted(run, outcome)
    _stated(run)
    _kept(run, report)
    _timed(run, export)
    _unverified(run, outcome, export)
    _against_tool(run)
    if run.broken and broken is not None:
        _broken(run, broken)


def _ours(outcome):
    """What the program's export came to, as the report has it."""
    told = Told()
    for key, into in (("written", "written"), ("failed", "failed"), ("without_drawing_no", "shells"),
                      ("pid_entries", "pid"), ("templates", "templates")):
        if _whole(outcome.get(key)):
            setattr(told, into, outcome[key])
    if isinstance(outcome.get("unmatched"), list):
        told.unmatched = len(outcome["unmatched"])
    if isinstance(outcome.get("warnings"), list):
        told.warnings = [str(line) for line in outcome["warnings"]]
    gate = outcome.get("gate")
    if isinstance(gate, dict):
        counts = [gate.get(key) for key in ("clean", "dirty", "revision_mismatch", "unpaired")]
        told.gate = tuple(counts) if all(_whole(count) for count in counts) else None
        told.gate_lines = [str(line) for line in gate.get("lines") or []]
    return told


def _meets(actual, expected):
    op, value = expected
    if actual is None:
        return False
    return actual > value if op == ">" else actual == value


def _shown(value):
    if value is None:
        return "—"
    return "/".join(str(part) for part in value) if isinstance(value, tuple) else str(value)


def _counted(run, outcome):
    """The numbers of the export, held to the expectation."""
    ours = run.ours
    others = [name for name in run.files if _kind(name) is None]
    actual = {
        "files": len(run.files),
        "written": ours.written,
        "failed": ours.failed,
        "pid": ours.pid,
        "templates": ours.templates,
        "unmatched": ours.unmatched,
        "shells": ours.shells,
        "unwritten": len(run.unwritten),
        "gate": ours.gate,
    }
    run.cells.update(actual)
    run.cells["others"] = others
    for key, expected in run.expect.items():
        if not _meets(actual[key], expected):
            run.failing.add(key)
            op, value = expected
            run.reasons.append(f"{EXPECTED[key]} {_shown(actual[key])}，期望 {op if op == '>' else ''}{_shown(value)}")
    if others:
        run.failing.add("files")
        run.reasons.append(f"导出目录里有不是导出写的文件：{_listing(others, '个')}")
    if _whole(outcome.get("files")) and outcome["files"] != len(run.files):
        run.failing.add("files")
        run.reasons.append(f"导出说写了 {outcome['files']} 个文件，目录里有 {len(run.files)} 个")


def _stated(run):
    """Whether every `_Data.xml` and `_Meta.xml` of the export states the plant name."""
    stated = f'Plant="{run.plant}"'.encode("utf-8")
    files = [name for name in run.files if _kind(name) in ("data", "meta")]
    lacking = []
    for name in files:
        try:
            if stated not in (run.folder / EXPORT / name).read_bytes():
                lacking.append(name)
        except OSError:
            lacking.append(name)
    run.cells["stated"] = (len(files) - len(lacking), len(files))
    if lacking:
        run.failing.add("stated")
        run.reasons.append(f"没有写工厂名 `{_cell(run.plant)}` 的文件 {_listing(lacking, '个')}")


def _kept(run, report):
    """The backup and its restore as they were: the files, lengths and times the test listed."""
    listed = report.get("kept") if isinstance(report.get("kept"), list) else []
    kept = [entry for entry in listed if isinstance(entry, dict)]
    if not any(entry.get("what") in ("source", "package") for entry in kept):
        run.reasons.append("报告里没有包在导出前后的样子")
    same, changed = [], []
    for entry in kept:
        what = KEPT.get(entry.get("what"), _cell(str(entry.get("what"))))
        before, after = entry.get("before"), entry.get("after")
        files = entry.get("changed") if isinstance(entry.get("changed"), list) else None
        counted = all(isinstance(side, dict) and _whole(side.get("files")) for side in (before, after))
        if not counted or files is None:
            changed.append(f"{what}没有导出前后的样子")
        elif before != after or files:
            said = f"：{_listing([str(name) for name in files], '个文件')}" if files else ""
            changed.append(f"{what}变了（文件 {before['files']} → {after['files']}{said}）")
        else:
            same.append(f"{what} {before['files']:,} 个文件")
    if kept:
        run.cells["kept"] = (same, changed)
    if changed:
        run.failing.add("kept")
    run.reasons += changed


def _timed(run, export):
    """How long the export took, from the command to its end, held to the limit when there is one."""
    seconds = export.get("seconds")
    if not _number(seconds):
        run.reasons.append("报告里没有导出的用时")
        return
    run.cells["seconds"] = seconds
    if run.limit is not None and seconds > run.limit:
        run.failing.add("seconds")
        run.reasons.append(f"导出用了 {seconds:.0f} 秒，超过 {run.limit:.0f} 秒")


def _unverified(run, outcome, export):
    """Whether the summary of a SQL Server package says that what it writes has not been verified."""
    if not outcome.get("sql_server"):
        return
    ended = export.get("ended") if isinstance(export.get("ended"), list) else []
    said = any(UNVERIFIED in str(line) for line in ended)
    run.cells["unverified"] = said
    if not said:
        run.reasons.append("SQL Server 包的结果里没有说「没有验收过」")


def _theirs(run):
    """What `pid_publish_xml` says of the package: Told, or None when it cannot be read."""
    tool = run.tool
    if tool is None or tool.exit_code is None:
        run.reasons.append("没有跑 pid_publish_xml")
        return None
    if tool.exit_code not in (0, 1):
        run.reasons.append(f"pid_publish_xml 失败（退出码 {tool.exit_code}）")
    try:
        stderr = tool.stderr.read_text(encoding="utf-8", errors="replace")
        stdout = tool.stdout.read_text(encoding="utf-8", errors="replace")
    except OSError as error:
        run.reasons.append(_cell(f"pid_publish_xml 的输出读不了（{error.strerror or error}）"))
        return None
    told = Told()
    for line in stderr.splitlines():
        line = line.rstrip()
        if line.startswith(WARNING):
            told.warnings.append(line[len(WARNING):])
        elif found := PUBLISHED.fullmatch(line):
            told.written, told.failed, told.shells = int(found[1]), int(found[3]), int(found[4])
        elif found := GATE.fullmatch(line):
            told.gate = tuple(int(found[at]) for at in (3, 4, 5, 6))
        elif found := GATE_NONE.fullmatch(line):
            told.gate = (0, 0, int(found[3]), int(found[4]))
    if told.written is None:
        run.reasons.append("pid_publish_xml 没有报写出几张（stderr 里没有 Published 那一行）")
    if told.gate is not None:
        told.gate_lines = [line.rstrip() for line in stdout.splitlines() if line.strip()]
    index, why = _load(tool.folder / INDEX)
    if isinstance(index, dict):
        told.pid = index.get("pid_entries") if _whole(index.get("pid_entries")) else None
        told.templates = index.get("template_pid_entries") if _whole(index.get("template_pid_entries")) else None
        unmatched = index.get("unmatched_pid_entries")
        told.unmatched = len(unmatched) if isinstance(unmatched, list) else None
    else:
        run.reasons.append(f"pid_publish_xml 的 {INDEX}：{_cell(why or '不是一个对象')}")
    return told


def _against_tool(run):
    """The export beside `pid_publish_xml`'s: the numbers both say, then every file, byte for byte."""
    theirs = _theirs(run)
    if theirs is None:
        return
    run.theirs = theirs
    ours = run.ours
    differ = []
    for key, label in (("written", "写出"), ("failed", "失败计数"), ("shells", "没有 DrawingNo"),
                       ("pid", ".pid 数"), ("templates", "图框模板"), ("unmatched", "没对上的 .pid")):
        if getattr(ours, key) != getattr(theirs, key):
            differ.append(f"{label} {_shown(getattr(ours, key))} / {_shown(getattr(theirs, key))}")
    if ours.warnings != theirs.warnings:
        differ.append(f"提醒 {len(ours.warnings)} 句 / {len(theirs.warnings)} 句" + ("，内容不同" if len(ours.warnings) == len(theirs.warnings) else ""))
    if ours.gate != theirs.gate:
        differ.append(f"比对结论 {_shown(ours.gate)} / {_shown(theirs.gate)}")
    elif ours.gate is not None and ours.gate_lines != theirs.gate_lines:
        differ.append(f"逐张的比对 {len(ours.gate_lines)} 行 / {len(theirs.gate_lines)} 行，内容不同")
    run.cells["numbers"] = differ
    if differ:
        run.failing.add("numbers")
        run.reasons.append("和 pid_publish_xml 报的不同（本程序 / 它）：" + "；".join(differ))

    try:
        run.their_files = _held(run.tool.folder)
    except OSError as error:
        run.reasons.append(_cell(f"pid_publish_xml 的目录读不了（{error.strerror or error}）"))
        return
    mine, other = run.files, run.their_files
    names = sorted(mine.keys() | other.keys())
    alone = [name for name in names if name not in other]
    absent = [name for name in names if name not in mine]
    unread = []
    for name in names:
        if name in mine and name in other and mine[name] != other[name]:
            try:
                one = (run.folder / EXPORT / name).read_bytes()
                two = (run.tool.folder / name).read_bytes()
            except OSError:
                unread.append(name)
                continue
            run.differing.append((name, len(one), len(two), _first_difference(one, two)))
    same = {name for name in names if mine.get(name) == other.get(name)}
    by_kind = {}
    for name in names:
        group = {"data": "data", "meta": "data", "coverage": "index", "index": "index"}.get(_kind(name), "other")
        held = by_kind.setdefault(group, [0, 0])
        held[0] += name in same
        held[1] += 1
    run.cells["bytes"] = (len(same), len(names), by_kind)
    if run.differing or alone or absent or unread:
        run.failing.add("bytes")
    if run.differing:
        run.reasons.append(f"和 pid_publish_xml 字节不同的文件 {len(run.differing)} 个（{_listing([found[0] for found in run.differing], '个')}）")
    if alone:
        run.reasons.append(f"只有本程序写出的文件 {_listing(alone, '个')}")
    if absent:
        run.reasons.append(f"只有 pid_publish_xml 写出的文件 {_listing(absent, '个')}")
    if unread:
        run.reasons.append(f"读不了、没法比的文件 {_listing(unread, '个')}")


def _broken(run, broken):
    """The drawing made into none: named with a reason, no file in the index, the others written."""
    drawing, archive, entry = (str(broken.get(key) or "") for key in ("drawing", "archive", "entry"))
    index, why = _load(run.folder / EXPORT / INDEX)
    drawings = index.get("drawings") if isinstance(index, dict) else None
    if not isinstance(drawings, list):
        run.reasons.append(f"本程序的 {INDEX}：{_cell(why or '没有 drawings 一项')}")
        return
    drawings = [entry for entry in drawings if isinstance(entry, dict)]
    hit = [one for one in drawings if one.get("zip_file") == archive and one.get("zip_entry") == entry]
    others = [one for one in drawings if one not in hit]
    listed = [one for one in hit if one.get("files")]
    reason = next((why for name, why in run.unwritten if name == drawing), None)
    left = [str(one.get("name")) for one in others if not one.get("files")]
    run.cells["broken"] = (drawing, archive, entry, reason, len(others) - len(left), len(others))
    if not drawing or len(hit) != 1:
        run.reasons.append(f"{INDEX} 里 `{_cell(archive)}` 的 `{_cell(entry)}` 有 {len(hit)} 张图")
    if listed:
        run.reasons.append(f"换坏的那张 {_name(drawing)} 在 {INDEX} 里还列着文件")
    if not reason:
        run.reasons.append(f"没写出的名单里没有换坏的那张 {_name(drawing)}，或者没有原因")
    if left:
        run.reasons.append(f"没换坏的图也没写出：{_listing(left)}")


def _row(run):
    """The table row of one export."""

    def cell(key, shown, *failing):
        if key not in run.cells:
            return "—"
        text = shown(run.cells[key])
        return text + "（不过）" if run.failing & {key, *failing} else text

    def pid(_):
        return f"{_shown(run.cells['pid'])} / {_shown(run.cells['templates'])} / {_shown(run.cells['unmatched'])}"

    def files(count):
        others = run.cells.get("others") or []
        return str(count) + (f"（另有 {len(others)} 个不是导出写的）" if others else "")

    def plant(stated):
        good, of = stated
        return f"`{_cell(run.plant)}`（{good} / {of}）"

    def numbers(differ):
        return "不同：" + "；".join(differ) if differ else "相同"

    def same_bytes(value):
        same, total, by_kind = value
        parts = []
        if "data" in by_kind:
            parts.append("`_Data`、`_Meta` {} / {}".format(*by_kind["data"]))
        if "index" in by_kind:
            parts.append("`_Coverage`、`_Index` {} / {}".format(*by_kind["index"]))
        if "other" in by_kind:
            parts.append("别的文件 {} / {}".format(*by_kind["other"]))
        return f"{same} / {total}" + ("（" + "；".join(parts) + "）" if parts else "")

    def kept(value):
        same, changed = value
        return "变了" if changed else "不变（" + "；".join(same) + "）"

    cells = [
        f"`{_cell(run.name)}`",
        _label(run) + ("，没有验收过" if run.package == UNACCEPTED else ""),
        cell("stated", plant),
        cell("written", str),
        cell("failed", str),
        cell("pid", pid, "templates", "unmatched"),
        cell("shells", str),
        cell("files", files),
        cell("numbers", numbers),
        cell("bytes", same_bytes),
        cell("kept", kept),
        cell("seconds", lambda seconds: f"{seconds:.1f} s"),
        f"`{_cell(run.expect_text)}`",
        "不过：" + "；".join(run.reasons) if run.reasons else "过",
    ]
    return "| " + " | ".join(cells) + " |"


def _forms(run, other):
    """The line of one backup exported from its folder and from its zip, and whether the two agree."""
    label = _package(run.package)
    lacking = [FORMS[side.form] for side in (run, other) if side.files is None]
    if lacking:
        which = "和".join(lacking)
        gap = " " if which[-1].isascii() else ""
        return f"- {label}：没法比，{which}{gap}那一次没有读到导出的文件。", False

    def drawn(side):
        return {name: held for name, held in side.files.items() if _kind(name) in ("data", "meta")}

    one, two = drawn(run), drawn(other)
    found = []
    for side, mine, theirs in ((run, one, two), (other, two, one)):
        alone = sorted(name for name in mine if name not in theirs)
        if alone:
            form = FORMS[side.form]
            where = f" {form} " if form.isascii() else form
            found.append(f"只在{where}那次里的 {_listing(alone, '个')}")
    changed = sorted(name for name in one if name in two and one[name] != two[name])
    if changed:
        found.append(f"字节不同的 {_listing(changed, '个')}")
    if found:
        return f"- {label}：不同。" + "；".join(found) + "。", False
    totals = f"目录那次 {len(run.files)} 个文件，zip 那次 {len(other.files)} 个"
    return f"- {label}：相同。{len(one)} 个 `_Data.xml`、`_Meta.xml`，名单相同，逐字节相同（{totals}）。", True


def _plants(run, other):
    """The line of one backup exported under the default plant name and under another: whether the
    `_Data.xml` and `_Meta.xml` differ in the plant name and in nothing else."""
    label = _package(run.package)
    if run.files is None or other.files is None:
        return f"- {label}：没法比，有一次没有读到导出的文件。"
    stated, said = f'Plant="{run.plant}"'.encode("utf-8"), f'Plant="{other.plant}"'.encode("utf-8")
    names = sorted(name for name in run.files if _kind(name) in ("data", "meta") and name in other.files)
    unlike = same = beyond = 0
    for name in names:
        try:
            one = (run.folder / EXPORT / name).read_bytes()
            two = (other.folder / EXPORT / name).read_bytes()
        except OSError:
            beyond += 1
            continue
        if one == two:
            same += 1
        elif one.replace(stated, said) == two:
            unlike += 1
        else:
            beyond += 1
    told = f"{len(names)} 个 `_Data.xml`、`_Meta.xml` 里，{unlike} 个只有工厂名不同"
    if same:
        told += f"，{same} 个完全相同"
    if beyond:
        told += f"，{beyond} 个换掉工厂名以后还不同"
    return f"- {label}：`{_cell(run.plant)}` 和 `{_cell(other.plant)}` 两次导出，{told}。"


def _gates(run):
    """The lines of one export against a reference folder: the comparison of both sides."""
    ours, theirs = run.ours, run.theirs
    if ours is None:
        return [f"- {_label(run)}：本程序没有导出的结果。"]
    shown = "张一致、张有差异、张的参考是另一版、张没有参考".split("、")

    def said(gate):
        if gate is None:
            return "没有比对结论"
        return "，".join(f"{count} {what}" for count, what in zip(gate, shown))

    out = [f"- {_label(run)}，参考目录 `{_cell(run.reference)}`：本程序 {said(ours.gate)}。"]
    if theirs is not None:
        agree = ours.gate == theirs.gate and ours.gate_lines == theirs.gate_lines
        lines = f"逐张的比对 {len(ours.gate_lines)} 行，" + ("逐行相同" if agree else "不同")
        out.append(f"  `pid_publish_xml --diff-against`：{said(theirs.gate)}；{lines}。")
    return out


def _brokenly(run):
    """The line of the copy with a broken drawing."""
    if "broken" not in run.cells:
        return f"- {_label(run)}：没有读到换坏的那张。"
    drawing, archive, entry, reason, written, others = run.cells["broken"]
    because = f"原因：{_reason(reason)}" if reason else "没有原因"
    failed = run.cells.get("failed")
    return (
        f"- {_label(run)}：把 `{_cell(archive)}` 里的 `{_cell(entry)}` 换成不是图的内容。"
        f"它是 {_name(drawing or '（没找到）')}，在没写出的名单里，{because}。"
        f"其余 {others} 张写出 {written} 张，失败计数 {_shown(failed)}。"
    )


def _built(exports):
    """Whether the tests ran with debug assertions, as far as their reports say."""
    seen = {run.debug for run in exports}
    seen.discard(None)
    if not seen:
        return ""
    return "debug_assertions " + ("开" if seen == {True} else "关" if seen == {False} else "有开有关")


def summary(exports, partial=False, said=None):
    """The Markdown of the exports: (its lines, what does not pass).

    `failed` holds one sentence per export that does not pass and per comparison
    that does not agree, and is empty when everything passes.
    """
    said = said or {}
    out, failed = ["**publish 文件比对汇总**", ""], []
    if partial:
        out += ["这次只跑了一部分，不是完整的验收。", ""]
    if not exports:
        return [*out, "结论：什么也没有跑。"], ["什么也没有跑"]

    built = "，".join(part for part in (f"{said['build']}（cargo 的原话）" if said.get("build") else "", _built(exports)) if part)
    if built:
        out += [f"构建：{built}。", ""]
    if said.get("tools"):
        out += [f"命令行工具：`{said['tools']}`。", ""]

    out += [
        "| 导出 | 包 | 工厂名（写进文件的 / `_Data`、`_Meta`） | 写出 | 失败计数 | `.pid` 共 / 图框模板 / 没对上 "
        "| 没有 DrawingNo | 文件 | 和 `pid_publish_xml` 报的数 | 和 `pid_publish_xml` 逐字节相同 / 共 "
        "| 包和还原目录 | 用时 | 期望值 | 结果 |",
        "|---|---|---|---:|---:|---|---:|---|---|---|---|---:|---|---|",
    ]
    for run in exports:
        out.append(_row(run))
        if run.reasons:
            failed.append(f"{_label(run)}：" + "；".join(run.reasons))
    notes = (
        "期望值写成 `键=数` 或 `键>数`，键有 "
        + "、".join(f"{key}（{label}）" for key, label in EXPECTED.items())
        + "；gate 的四个数是一致、有差异、参考是另一版、没有参考。不合期望的格子标「不过」。"
        "「报的数」比本程序的结果和 `pid_publish_xml` 报的：写出、失败计数、没有 DrawingNo 取它 stderr 的 Published 那一行，"
        "`.pid` 的三个数取它写的 `_Index.json`，提醒取它的 warning 行，比对结论取它的 Package gate 那一行和 stdout。"
        "「逐字节相同」比两个目录里的全部文件；两边交给整包入口的是同一个包路径，所以 `_Coverage.json`、`_Index.json` 里记的路径也相同。"
        "「包和还原目录」比导出前后的文件名单、大小和修改时间。"
        "用时从发出命令算到导出结束。"
    )
    for run in exports:
        if run.limit is not None:
            notes += f"{_label(run)}的导出不能超过 {run.limit:.0f} 秒。"
    timed = [run.seconds for run in exports if run.seconds is not None]
    if timed:
        notes += f"连同打开、导出、列目录以及 cargo 自己花的时间，{len(timed)} 次测试{_spent(sum(timed))}"
        tooled = [run.tool.seconds for run in exports if run.tool and run.tool.seconds is not None]
        if tooled:
            notes += f"；`pid_publish_xml` {len(tooled)} 次{_spent(sum(tooled))}"
        notes += "。"
    out += ["", notes, ""]
    if any(run.package == UNACCEPTED for run in exports):
        unverified = [run for run in exports if run.package == UNACCEPTED and run.cells.get("unverified")]
        out += [
            "SQL Server 包没有验收过：这一行只说明它导出了多少、和命令行工具是否一致，写出的内容没有和任何参考比过。"
            + ("本程序的结果里有「没有验收过」那一句。" if unverified else ""),
            "",
        ]

    # ── From the folder and from the zip ───────────────────────────────────────────────────────────
    plain = [run for run in exports if not run.broken and not run.reference]
    by = {(run.package, run.form, run.plant): run for run in plain}
    pairs = [(run, by[run.package, "zip", run.plant]) for run in plain if run.form == "folder" and (run.package, "zip", run.plant) in by]
    if pairs:
        out += ["同一个包从目录打开和从 zip 打开（`_Data.xml`、`_Meta.xml` 的名单和字节都要相同；`_Coverage.json`、`_Index.json` 里记着包的路径，不比）：", ""]
        for run, other in pairs:
            line, same = _forms(run, other)
            out.append(line)
            if not same:
                failed.append(f"目录和 zip 的比较：{_package(run.package)}")
        out.append("")

    # ── Under another plant name ─────────────────────────────────────────────────────────────────
    named = [
        (by[run.package, run.form, DEFAULT_PLANT], run)
        for run in plain
        if run.plant != DEFAULT_PLANT and (run.package, run.form, DEFAULT_PLANT) in by
    ]
    if named:
        out += ["同一个包换一个工厂名（只是读数，不算进结论；工厂名有没有交对看表里的「工厂名」一格和「逐字节相同」一格）：", ""]
        out += [_plants(run, other) for run, other in named]
        out.append("")

    # ── Against a reference folder ───────────────────────────────────────────────────────────────
    gated = [run for run in exports if run.reference]
    if gated:
        out += ["和参考目录比对（两边的结论和逐张的比对都要相同，在表里的「报的数」一格）：", ""]
        for run in gated:
            out += _gates(run)
        out.append("")

    # ── The copy with a broken drawing ───────────────────────────────────────────────────────────
    broken = [run for run in exports if run.broken]
    if broken:
        out += ["换坏一张：", ""]
        out += [_brokenly(run) for run in broken]
        out.append("")

    # ── What the command line said ───────────────────────────────────────────────────────────────
    spoken = next((run for run in exports if isinstance(run.report, dict) and isinstance(run.report.get("export"), dict)), None)
    if spoken is not None:
        export = spoken.report["export"]
        lines = []
        for key, what in (("started", "命令返回时"), ("status", "导出中不带参数"), ("ended", "导出结束时")):
            listed = export.get(key) if isinstance(export.get(key), list) else []
            lines += [f"{what}：{_cell(str(text))}" for text in listed]
        if lines:
            out += [f"命令行上说的话（{_label(spoken)}）：", "", "```text", *lines, "```", ""]

    # ── The files that differ, and the drawings not written ──────────────────────────────────────
    out += ["和 `pid_publish_xml` 字节不同的文件：", ""]
    differing = [
        f"- {_label(run)} {_name(name)}：本程序 {ours:,} 字节，`pid_publish_xml` {theirs:,} 字节，第 {at:,} 个字节起不同。"
        for run in exports
        for name, ours, theirs, at in run.differing
    ]
    out += differing or ["- 没有。"]
    out += ["", "本程序没写出的图和它给的原因：", ""]
    unwritten = [
        f"- {_label(run)} {_name(name)}：{_reason(why) if why.strip() else '（没有写原因）'}"
        for run in exports
        for name, why in run.unwritten
    ]
    out += unwritten or ["- 没有。"]
    out.append("")

    if failed:
        out.append(f"结论：{len(failed)} 处不过。")
        out += [f"- {what}" for what in failed]
    else:
        agreed = f"，{len(pairs)} 项目录和 zip 的比较相同" if pairs else ""
        out.append(f"结论：{len(exports)} 次整包导出全部通过{agreed}。")
    return out, failed


def main(argv=None):
    parser = argparse.ArgumentParser(
        prog="export_compare.py",
        description="Sum up the exports that export-compare.ps1 ran on the sample backups.",
    )
    parser.add_argument("runs", help="the runs.json that export-compare.ps1 wrote")
    parser.add_argument("--out", help="also write the Markdown to this file, as UTF-8")
    try:
        args = parser.parse_args(argv)
    except SystemExit as stop:
        return 2 if stop.code else 0

    # A console that cannot show Chinese must not turn a verdict into a crash.
    for stream in (sys.stdout, sys.stderr):
        if hasattr(stream, "reconfigure"):
            stream.reconfigure(errors="backslashreplace")
    try:
        exports, partial, said = load_runs(args.runs)
    except ManifestError as error:
        print(f"export_compare: {error}", file=sys.stderr)
        return 2
    for run in exports:
        read_export(run)
    lines, failed = summary(exports, partial, said)
    text = HOME.sub("%USERPROFILE%", "\n".join(lines) + "\n")
    if args.out:
        try:
            Path(args.out).write_text(text, encoding="utf-8", newline="\n")
        except OSError as error:
            print(f"export_compare: {args.out}: 写不了（{error.strerror or error}）", file=sys.stderr)
            return 2
    sys.stdout.write(text)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
