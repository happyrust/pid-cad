#!/usr/bin/env python3
"""校验与交付面板的端到端脚本（PNL-13；SPEC-pid-verify-deliver-panel §7 「端到端」那一行）。

起真实程序，只经它的本机 REST 通道（`--http`）驱动：不模拟鼠标，不靠截图。脚本先在仓库目录里
`cargo build --offline --locked --bin OpenCADStudio`，再把 cargo 报出的那个可执行文件（`cargo run` 起的也是它）
起在单独的隐藏桌面上（仓库的 `docs/automation/hidden_desktop.py`），窗口不出现在人正在用的桌面上，
跑的时候机器可以照常用。外壳和经典界面各起一次，每次：

1. 造图：从仓库里的小图起步，另存一份空白图，复制成项目目录里的 `PID-1101.dwg` 并打开，画泵、阀门、
   容器、管线和仪表，存回原处。
2. 显示面板：`ui_bounds` 按前缀 `pid-verify-` 读得到面板和三个页签（成功标准第 1 条）。
3. 切页签，检查当前图：面板的计数、违规列表与 `pid_check` 的结果一致（第 2、3 条）。
4. 生成当前图的材料清单，从面板导出 CSV：面板的行数与 `pid_bom` 一致，文件在，行数是表头加清单的
   行数（第 8 条）。
5. 往项目目录复制空白图（默认 3,000 份），检查全项目：读到 `busy.done`、`busy.total`，再读时读了的变多了；
   这时再点【检查当前图】被拒（`busy`）；取消后原来的检查结果不动（第 12 条）。
6. 隐藏面板：`pid-verify-` 开头的位置一个也读不到（第 1、13 条）。

    python scripts/pid-verify-e2e/run.py [--out <仓库外的目录>] [--port 8090] [--ui both|shell|classic] [--copies 3000]

结果写在 `<输出目录>/logs/pid-verify.result.txt`：每项一行 PASS 或 FAIL，最后是合计和桌面有没有被
动过。全部通过时退出码 0，有失败时 1，跑不起来（输出目录在仓库里、找不到 cargo、编译失败）时 2。
"""

import argparse
import csv
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import traceback
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
sys.dont_write_bytecode = True
sys.path.insert(0, str(REPO / "docs" / "automation"))
import hidden_desktop as hd  # noqa: E402

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8")

SCENARIO = "pid-verify"
PREFIX = "pid-verify-"
SHOWN = ("pid-verify-panel", "pid-verify-tab-check", "pid-verify-tab-bom", "pid-verify-tab-deliver")
COUNTS = ("错误", "警告", "提示", "已豁免")
INTERFACES = {"shell": [], "classic": ["--classic"]}
# PID-1101 里画的东西：规则检查和材料清单都有话可说。
PLANT = [
    ("PIDINSERT pump", [(0.0, 0.0)]),
    ("PIDINSERT gate_valve", [(-28.0, 0.0)]),
    ("PIDINSERT vessel", [(200.0, 12.0)]),
    ("PIDPIPE", [(200.0, 100.0), (300.0, 100.0)]),
    ("PIDPIPE run", [(14.0, 0.0), (200.0, 0.0)]),
    ("PIDFITTING gate_valve", [(100.0, 0.0)]),
    ("PIDFITTING transmitter", [(40.0, 0.0)]),
    ("PIDINSERT transmitter", [(58.0, 80.0)]),
]
FIRST_COPY = 2001
SETUP_FAILED = 2


class Results:
    """PASS/FAIL 行，写法同 `scripts/collab-e2e/`：写进 `<logs>/pid-verify.result.txt`，也打印出来。"""

    def __init__(self, logs):
        logs.mkdir(parents=True, exist_ok=True)
        self.path = logs / f"{SCENARIO}.result.txt"
        self.path.write_text(f"=== {SCENARIO} {time.strftime('%Y-%m-%dT%H:%M:%S')} ===\n", encoding="utf-8")
        self.rows = []
        # 跑之前人正在用的桌面什么样：指针在哪、哪个窗口在前台。
        self.desktop_before = hd.desktop_state()

    def write(self, line):
        print(line, flush=True)
        with open(self.path, "a", encoding="utf-8") as f:
            f.write(line + "\n")

    def note(self, text):
        self.write(f"      {text}")

    def check(self, assertion, ok, detail):
        self.rows.append((assertion, bool(ok)))
        self.write(f"{'PASS' if ok else 'FAIL'} {assertion} {detail}")
        return bool(ok)

    def summary(self):
        """合计。桌面那一项只在没人碰过机器、指针或前台窗口却变了时算失败。"""
        desktop = hd.same_desktop(self.desktop_before, hd.desktop_state())
        self.check("desktop", "NO input was registered" not in desktop, desktop)
        failed = sum(1 for _, ok in self.rows if not ok)
        self.write(f"=== {len(self.rows) - failed} passed, {failed} failed ===")
        return failed == 0


def built(cargo):
    """在仓库目录里编程序，返回 cargo 报出的可执行文件；编不出来时是 None。要在仓库目录里编：cargo 按
    工作目录找 `.cargo/config.toml`，在别处编会读不到仓库的配置，整棵依赖换参数重编，编出的程序也不一样。"""
    command = [cargo, "build", "--offline", "--locked", "--bin", "OpenCADStudio",
               "--message-format=json-render-diagnostics"]
    done = subprocess.run(command, cwd=REPO, stdout=subprocess.PIPE, text=True, encoding="utf-8")
    if done.returncode != 0:
        print(f"编译失败（退出码 {done.returncode}）", file=sys.stderr)
        return None
    for line in done.stdout.splitlines():
        try:
            message = json.loads(line)
        except ValueError:
            continue
        if (message.get("reason") == "compiler-artifact" and message.get("executable")
                and (message.get("target") or {}).get("name") == "OpenCADStudio"):
            return Path(message["executable"])
    print("cargo 没有报出 OpenCADStudio 的可执行文件", file=sys.stderr)
    return None


def port_free(port):
    with socket.socket() as probe:
        probe.settimeout(0.3)
        return probe.connect_ex(("127.0.0.1", port)) != 0


def said(answer):
    answer = answer or {}
    keys = ("ok", "status", "code", "error", "reason", "message")
    return json.dumps({key: answer[key] for key in keys if key in answer}, ensure_ascii=False)[:300]


def settled(path, timeout=60):
    """`path` 写出来了、大小不再变。"""
    deadline, sizes = time.time() + timeout, []
    while time.time() < deadline:
        if path.is_file():
            sizes.append(path.stat().st_size)
            if len(sizes) >= 3 and sizes[-1] > 0 and sizes[-1] == sizes[-2] == sizes[-3]:
                return True
        time.sleep(0.2)
    return False


def records(path):
    """CSV 的记录，表头算一条；文件头的 BOM 不算内容。"""
    if not path.is_file():
        return []
    with open(path, encoding="utf-8-sig", newline="") as f:
        return list(csv.reader(f))


def row_key(row):
    return (row.get("rule_id"), row.get("object_key"), row.get("drawing_id"))


def result_key(result):
    return (result.get("rule_id"), result.get("object_key"), (result.get("object") or {}).get("drawing_id"))


def points_at(row, results):
    """行的 `index` 指向 `results` 里同一条（规则、对象、图号）。"""
    index = row.get("index")
    return isinstance(index, int) and 0 <= index < len(results) and result_key(results[index]) == row_key(row)


def preconditions(state):
    """发布的四个前提（§3.2 `deliver.publish.preconditions`），里面的 `clean` 就是「没有未存的修改」。"""
    return ((state.get("deliver") or {}).get("publish") or {}).get("preconditions") or {}


def read_more(state, done):
    """还在读项目的图，读了的比 `done` 多、还没读完。"""
    busy = state.get("busy") or {}
    now, total = busy.get("done"), busy.get("total")
    return isinstance(now, int) and isinstance(total, int) and isinstance(done, int) and done < now < total


def check_shown(state, rows):
    """规则检查页签显示的那次检查：取消全项目检查前后拿来比。"""
    check = state.get("check") or {}
    keys = ("has_result", "scope", "checked_at", "rule_set_version", "stale", "total", "counts")
    return {key: check.get(key) for key in keys} | {"rows": [(row.get("index"),) + row_key(row) for row in rows]}


class Session:
    """一种界面里的一次程序：从造图到面板再隐藏。"""

    def __init__(self, results, ui, out, port, copies, exe):
        self.R, self.ui, self.port, self.copies, self.exe = results, ui, port, copies, exe
        self.out = out / ui
        self.project, self.later = self.out / "project", self.out / "later"
        self.first, self.blank = self.project / "PID-1101.dwg", self.later / "PID-0000.dwg"
        self.editor = None

    def label(self, number):
        return f"{self.ui}/§6.{number}"

    # ── 经 REST 通道 ──
    def request(self, op, fields=None):
        """当前图上的一个协议 1 操作，带上它此刻的修订号。"""
        state = self.editor.state()
        body = dict(fields or {})
        revision = (state.get("result") or state).get("revision")
        if revision is not None:
            body["revision"] = revision
        status, answer = self.editor.op(op, body)
        answer = answer or {}
        ok = status == 200 and answer.get("ok") is not False
        if not ok:
            self.R.note(f"[{self.ui}] {op} {json.dumps(fields or {}, ensure_ascii=False)}: HTTP {status} {said(answer)}")
        return ok, answer

    def press(self, value):
        """替人按面板上的一个按钮（§3.3）。"""
        status, answer = self.editor.op("action", {"name": "pid_verify", "value": value})
        answer = answer or {}
        self.R.note(f"[{self.ui}] action pid_verify {value!r}: HTTP {status} {said(answer)}")
        return status == 200 and answer.get("ok") is True, answer

    def panel(self, offset=0):
        return self.editor.call("GET", f"/pid/verify-panel?offset={offset}")[1] or {}

    def wait_panel(self, ready, timeout, every=0.3):
        deadline, state = time.time() + timeout, {}
        while time.time() < deadline:
            state = self.panel()
            if ready(state):
                return state
            time.sleep(every)
        self.R.note(f"[{self.ui}] 等了 {timeout} 秒，面板没到要的样子")
        return state

    def rows(self, total):
        """规则检查列表的全部行，照面板状态的分页一页页读。"""
        rows = []
        while len(rows) < total:
            page = (self.panel(len(rows)).get("check") or {}).get("rows") or []
            if not page:
                break
            rows.extend(page)
        return rows

    def wait_bounds(self, ready, timeout=15):
        deadline, found = time.time() + timeout, {}
        while time.time() < deadline:
            found = self.editor.bounds(PREFIX)
            if ready(found):
                return found
            time.sleep(0.2)
        return found

    # ── 各步 ──
    def run(self):
        for folder in (self.project, self.later):
            if folder.exists():
                shutil.rmtree(folder)
            folder.mkdir(parents=True)
        start = hd.start_drawing(self.out)
        if not port_free(self.port):
            self.R.check(f"{self.ui}/准备", False, f"端口 {self.port} 被占用：换一个（--port）")
            return
        args = [start, "--backend", "vulkan", "--new-instance", *INTERFACES[self.ui]]
        try:
            with hd.HiddenEditor(self.exe, self.port, self.out, args) as self.editor:
                self.editor.wait_ready()
                self.editor.find_window()
                if self.prepared():
                    self.shown()
                    self.checked()
                    self.listed()
                    self.cancelled()
                    self.hidden()
        except SystemExit as error:
            self.R.check(f"{self.ui}/准备", False, f"程序没起来或中途退出：{error}；看 {self.out / 'editor-stderr.log'}")
        except Exception as error:  # noqa: BLE001 - 记成一项失败，另一种界面照跑
            self.R.check(f"{self.ui}/脚本", False, f"跑到一半出了异常：{error!r}")
            for line in traceback.format_exc().splitlines():
                self.R.note(line)

    def prepared(self):
        """带路径的 `save` 只是另存一份，标签页还是原来那张：空白图这样存，复制成 PID-1101 再打开、在它上面
        画、存回原处。起步的图一笔不动，结束时关窗不会问要不要存。"""
        blank_ok, _ = self.request("save", {"path": str(self.blank)})
        blank_ok = blank_ok and settled(self.blank)
        if blank_ok:
            shutil.copyfile(self.blank, self.first)
        status, answer = self.editor.op("open", {"path": str(self.first)})
        state = self.wait_panel(lambda s: s.get("drawing_id") == "PID-1101", 60)
        opened = status == 200 and (answer or {}).get("ok") is not False and state.get("drawing_id") == "PID-1101"
        drawn = []
        for cmd, points in PLANT:
            ok, _ = self.request("run", {"cmd": cmd})
            fed = [self.request("input", {"kind": "point", "point": [x, y, 0.0], "space": "wcs"})[0]
                   for x, y in points]
            self.request("input", {"kind": "enter"})
            drawn.append(ok and all(fed))
        saved, _ = self.request("save")
        state = self.wait_panel(lambda s: preconditions(s).get("clean") is True, 30)
        clean = preconditions(state).get("clean") is True
        return self.R.check(
            f"{self.ui}/准备",
            blank_ok and opened and all(drawn) and saved and clean,
            f"另存空白图 later/{self.blank.name}：{blank_ok}；复制成 project/{self.first.name} 并打开：{opened}"
            f"（面板说当前图是 {state.get('drawing_id')}）；画 {sum(drawn)}/{len(PLANT)} 样；存回原处：{saved}，"
            f"没有未存的修改：{clean}",
        )

    def shown(self):
        ok, _ = self.press("show")
        found = self.wait_bounds(lambda found: all(key in found for key in SHOWN))
        visible = self.panel().get("visible")
        missing = [key for key in SHOWN if key not in found]
        self.R.check(
            self.label(1),
            ok and not missing and visible is True,
            f"显示面板后 ui_bounds 按前缀 {PREFIX} 读到 {len(found)} 个位置，面板和三个页签"
            f"{'都在' if not missing else '缺 ' + '、'.join(missing)}；visible={visible}",
        )
        for key in SHOWN:
            self.R.note(f"[{self.ui}] {key}: {found.get(key)}")

    def checked(self):
        went = []
        for tab in ("bom", "deliver", "check"):
            ok, _ = self.press(f"tab={tab}")
            went.append((tab, ok, self.panel().get("tab")))
        self.R.check(
            self.label(2),
            all(ok and now == tab for tab, ok, now in went),
            "依次点「材料清单」「交付」「规则检查」三个页签，面板状态的 tab 依次是 "
            + "、".join(str(now) for _, _, now in went),
        )

        ok, _ = self.press("check=drawing")
        state = self.wait_panel(
            lambda s: (s.get("check") or {}).get("scope") == "drawing" and s.get("busy") is None, 120)
        check = state.get("check") or {}
        rows = self.rows(check.get("total") or 0)
        status, answer = self.editor.call("POST", "/pid/check", {})
        answer = answer or {}
        results = answer.get("results") or []
        summary = answer.get("summary") or {}
        listed = sorted(row_key(row) for row in rows)
        open_results = sorted(result_key(result) for result in results if not result.get("waived"))
        counts = {name: (check.get("counts") or {}).get(name) for name in COUNTS}
        wanted = {name: summary.get(name) for name in COUNTS}
        self.R.check(
            self.label(3),
            ok and check.get("has_result") is True and status == 200 and answer.get("ok") is True
            and len(rows) == check.get("total") and rows and listed == open_results and counts == wanted,
            f"点【检查当前图】后面板列出 {len(rows)} 行（total {check.get('total')}），计数 {counts}；"
            f"pid_check（HTTP {status}）有 {len(results)} 条结果，没豁免的 {len(open_results)} 条，summary {wanted}；"
            f"两边按（规则、对象、图号）比：{'一致' if listed == open_results else '不一致'}",
        )
        for row in rows[:5]:
            self.R.note(f"[{self.ui}] 行 {row.get('index')}: {row.get('severity')} {row.get('rule_id')} "
                        f"{row.get('label')} {row.get('message')}")

        # pid_check 之后面板显示的就是这次的结果：行的 index 指向它 results[] 里的同一条。
        again = self.rows((self.panel().get("check") or {}).get("total") or 0)
        pointed = [points_at(row, results) for row in again]
        self.R.check(
            self.label(3),
            again and all(pointed) and sorted(row_key(row) for row in again) == open_results,
            f"pid_check 之后面板列出 {len(again)} 行，每行的 index 指向 pid_check results[] 里同一条"
            f"（规则、对象、图号）：{sum(pointed)}/{len(again)}",
        )

    def listed(self):
        tab_ok, _ = self.press("tab=bom")
        ok, _ = self.press("bom=drawing")
        state = self.wait_panel(
            lambda s: (s.get("bom") or {}).get("scope") == "drawing" and s.get("busy") is None, 120)
        bom = state.get("bom") or {}
        status, answer = self.editor.call("GET", "/pid/bom")
        answer = answer or {}
        rows = answer.get("rows") or []
        pipes = answer.get("pipe_summary") or []
        self.R.check(
            self.label(8),
            tab_ok and ok and bom.get("has_result") is True and bom.get("error") is None
            and rows and bom.get("total") == len(rows),
            f"点【当前图】后面板的清单 {bom.get('total')} 行；pid_bom（HTTP {status}）的 rows {len(rows)} 行，"
            f"管道汇总 {len(pipes)} 行",
        )

        target = self.out / "bom.csv"
        side = target.with_name("bom-pipe-summary.csv")
        for path in (target, side):
            path.unlink(missing_ok=True)
        ok, _ = self.press(f"bom_export={target.as_posix()}")
        written = settled(target) and settled(side)
        table, summary = records(target), records(side)
        notice = (self.panel().get("notice") or {}).get("text")
        self.R.check(
            self.label(8),
            ok and written and rows and len(table) == 1 + len(rows) and len(summary) == 1 + len(pipes),
            f"从面板导出 {target.name}：文件{'在' if target.is_file() else '不在'}，{len(table)} 条记录（应为表头加 "
            f"{len(rows)} 行）；{side.name} {len(summary)} 条（应为表头加 {len(pipes)} 行）；消息栏：{notice}",
        )
        if table:
            self.R.note(f"[{self.ui}] {target.name} 的表头：{table[0]}")

    def cancelled(self):
        self.press("tab=check")
        state = self.panel()
        before = check_shown(state, self.rows((state.get("check") or {}).get("total") or 0))
        for number in range(FIRST_COPY, FIRST_COPY + self.copies):
            shutil.copyfile(self.blank, self.project / f"PID-{number}.dwg")
        self.R.note(f"[{self.ui}] 往 project/ 复制了 {self.copies} 份 {self.blank.name}")

        ok, _ = self.press("check=project")
        state = self.wait_panel(
            lambda s: isinstance((s.get("busy") or {}).get("total"), int), 5, every=0.05)
        busy = state.get("busy") or {}
        done, total = busy.get("done"), busy.get("total")
        state = self.wait_panel(lambda s: read_more(s, done), 10, every=0.1)
        moved = (state.get("busy") or {}).get("done")
        self.R.check(
            self.label(12),
            ok and isinstance(done, int) and isinstance(total, int) and done < total and total >= self.copies
            and read_more(state, done),
            f"点【检查全项目】后 busy 是 {json.dumps(busy, ensure_ascii=False)}；再读，读了 {moved}，共 {total}："
            f"{'进度在走' if read_more(state, done) else '没看到进度往前走'}",
        )
        pressed, answer = self.press("check=drawing")
        self.R.check(
            self.label(12),
            not pressed and answer.get("code") == "PANEL_ACTION_UNAVAILABLE" and answer.get("reason") == "busy",
            f"还在读项目的图时再点【检查当前图】：{said(answer)}",
        )

        ok, _ = self.press("cancel")
        state = self.wait_panel(lambda s: s.get("busy") is None, 30)
        after = check_shown(state, self.rows((state.get("check") or {}).get("total") or 0))
        notice = (state.get("notice") or {}).get("text")
        self.R.check(
            self.label(12),
            ok and state.get("busy") is None and after == before,
            f"点【取消】后 busy={state.get('busy')}，消息栏：{notice}；规则检查页签显示的"
            f"{'还是原来那次' if after == before else '变了'}（范围 {before['scope']}、{before['total']} 行、计数 "
            f"{before['counts']}）",
        )
        time.sleep(2)
        state = self.panel()
        later = check_shown(state, self.rows((state.get("check") or {}).get("total") or 0))
        self.R.check(
            self.label(12),
            state.get("busy") is None and later == before,
            f"取消 2 秒后再读：没有事在做，规则检查页签{'仍是原来那次' if later == before else '变了'}",
        )

        left = []
        for number in range(FIRST_COPY, FIRST_COPY + self.copies):
            path = self.project / f"PID-{number}.dwg"
            for _ in range(20):
                try:
                    path.unlink(missing_ok=True)
                    break
                except PermissionError:
                    time.sleep(0.25)
            else:
                left.append(path.name)
        self.R.note(f"[{self.ui}] 复制的空白图删掉了，删不掉的：{left or '没有'}")

    def hidden(self):
        ok, _ = self.press("hide")
        found = self.wait_bounds(lambda found: not found)
        visible = self.panel().get("visible")
        self.R.check(
            self.label(13),
            ok and not found and visible is False,
            f"隐藏面板后 ui_bounds 按前缀 {PREFIX} 读到 {len(found)} 个位置"
            f"{'' if not found else '：' + '、'.join(sorted(found))}；visible={visible}",
        )


def main():
    parser = argparse.ArgumentParser(description="校验与交付面板的端到端脚本（经 --http 驱动真实程序）")
    parser.add_argument("--out", help="输出目录，必须在仓库之外。不给时用环境变量 OCS_E2E_EVIDENCE_DIR 下的 "
                                      "pid-verify，也没有时在系统临时目录下新建一个")
    parser.add_argument("--port", type=int, default=8090, help="程序的 REST 端口（默认 8090）")
    parser.add_argument("--ui", choices=("both", "shell", "classic"), default="both",
                        help="跑哪种界面（默认两种各跑一次）")
    parser.add_argument("--copies", type=int, default=3000,
                        help="检查全项目前往项目目录复制几份空白图（默认 3000）")
    args = parser.parse_args()

    if args.out:
        out = Path(args.out)
    elif os.environ.get("OCS_E2E_EVIDENCE_DIR"):
        out = Path(os.environ["OCS_E2E_EVIDENCE_DIR"]) / SCENARIO
    else:
        out = Path(tempfile.gettempdir()) / f"ocs-pid-verify-e2e-{time.strftime('%Y%m%d-%H%M%S')}"
    out = out.resolve()
    if out == REPO or REPO in out.parents:
        print(f"输出目录 {out} 在仓库里面：证据不留在仓库里，换一个仓库之外的目录", file=sys.stderr)
        return SETUP_FAILED
    cargo = shutil.which("cargo")
    if not cargo:
        print("路径上找不到 cargo", file=sys.stderr)
        return SETUP_FAILED
    print(f"输出目录：{out}", flush=True)
    exe = built(cargo)
    if not exe:
        return SETUP_FAILED
    print(f"程序：{exe}", flush=True)

    results = Results(out / "logs")
    for ui in ("shell", "classic") if args.ui == "both" else (args.ui,):
        Session(results, ui, out, args.port, args.copies, exe).run()
    passed = results.summary()
    print(f"结果：{results.path}", flush=True)
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
