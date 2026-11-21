# -*- coding: utf-8 -*-
"""OS-17、OS-18（SPEC-collab-offline-sync.md §4.4、§4.5、§5.6、§7、§8）：未同步清单、导出、面板上待同步的一行、退出提示。

准备：服务器（3240，新库），u1（3210）打开种子图 A 加入，在线改 P-1001 的设计压力为 1.0MPa、放一个
阀门 V（修订 2），打开协同面板。

OS-18（面板）：在线、队列空时面板上没有待同步那一行。杀掉服务器，u1 离线改 P-1001 的设计温度、V 的
备注（排队两步）：状态 connected=false、queued_ops=2，面板上有这一行（截图）。面板上的字控制接口读不到：
读的是这一行在不在（ui_bounds 里的 pid-collab-sync）和算这一行用的两个数（connected、queued_ops）；
字本身（「离线，2 步待同步」）由程序内的测试按同样两个数核对（OFF-10），另留截图。

OS-17（只有排队的）：PIDCOLLAB UNSYNCED 的第一行是「排队 2 条，被拒 0 条，没排上队 0 条」。

OS-18（退出）：保存后 QUIT：问「修改未同步」（界面状态里打开的对话框有 CollabUnsynced。对话框都是
工具窗，两次更新之间停在 tool_windows 里，modal 通常是空的）；答「取消」留下，对话框关掉，队列还是两步；
再 QUIT 再问一次，答「退出」后退出。

OS-18（归零）：服务器用同一个库起来，一个 WS 会话（u2）占住 P-1001；u1 重开 A：设计温度那步被拒
（CLAIM_HELD）并撤回，备注那步提交；队列为 0，面板上这一行没了（按 OFF-10 的做法，归零就是这一行消失）；
保存后 QUIT 不问，直接退出。

OS-17（三种都有）：杀掉服务器，u1 重开 A，离线改 V 的设计温度（排队一步），保存、退出；A 复制成 B，
不连协同的实例在 B 上改 V 的材质并保存（没排上队，记一条）。u1 打开 B：PIDCOLLAB UNSYNCED 的第一行
「排队 1 条，被拒 1 条，没排上队 1 条」，下面三条各一行；PIDCOLLAB EXPORT 导出的文件 entries 按状态各 1 条，
与命令行和状态接口（queued_ops、unsynced_count）的数对得上；保存、退出后不连协同的实例读 B：队列是
导出里那条排队的，清单是被拒和没排上队的两条。

证据：logs/o06-*.json、logs/gui-o06-*.err.log、logs/o06-srv.out.log、logs/ws-o06-*.jsonl，
shots/offline-o06-sync.png、shots/offline-o06-zero.png。
"""
import sys
import time

import m7h
import oh
import yh

R = m7h.Results("o06_unsynced_exit")
PRESSURE, TEMPERATURE, NOTE, MATERIAL = "设计压力", "设计温度", "备注", "材质"
SYNC, PANEL, ASKED = "pid-collab-sync", "pid-collab-panel", "CollabUnsynced"


def started(name, port, dwg, **more):
    gui = m7h.Gui(name, port, dwg=dwg, **more).start()
    plain = "（不连协同）" if more.get("collab") is False else ""
    R.check("setup", gui.wait_ready(180) is not None, f"{name}{plain} 打开 {dwg.name}")
    return gui


def finish(gui, what):
    saved = oh.save(gui)
    gone, how = oh.quit_gui(gui)
    R.check("setup", saved[0] and gone, f"{what}：保存（{saved[1]}）、退出（{how}）")


def claim(ws, tag, seconds=10):
    ws.send({"type": "claim", "session_id": ws.session_id, "tag": tag})
    return ws.wait_for(lambda m: m.get("type") in ("claim_granted", "claim_denied") and m.get("tag") == tag, seconds)


def panel(gui, shot=None):
    """The ids the collaboration panel drew in its last frame (after one more frame)."""
    gui.run("PIDCOLLAB ON")
    time.sleep(0.5)
    gui.capture(shot or (m7h.WORK / f"{gui.name}-frame.png"))
    return gui.ui_bounds("pid-collab")


def modal(gui):
    """ASKED while the exit question is open, else None. A dialog is a tool window: between
    updates it is parked in `tool_windows`, and `modal` names it only while one of its own
    messages runs."""
    state = gui.state()
    open_now = list(state.get("tool_windows") or []) + [state.get("modal")]
    return ASKED if ASKED in open_now else None


def quit_asked(gui, seconds=6):
    """QUIT; the exit question, if it comes up within `seconds` (None when it went away instead)."""
    try:
        gui.run("QUIT")
    except Exception:  # noqa: BLE001 — it may be gone before it answers
        pass
    deadline = time.time() + seconds
    while time.time() < deadline:
        if not gui.running():
            return None
        try:
            asked = modal(gui)
        except Exception:  # noqa: BLE001
            asked = None
        if asked:
            return asked
        time.sleep(0.2)
    return None


def answer(gui, name):
    return gui.api("POST", "/run", {"document_id": gui.doc_id(), "op": "action", "name": name})


def unmarked(line):
    """A command-line entry without the mark an information line starts with."""
    return line[2:] if line.startswith("ⓘ ") else line


def listing(gui):
    """PIDCOLLAB UNSYNCED: (its first line, the indented lines under it, stripped)."""
    before = len(oh.told(gui))
    gui.run("PIDCOLLAB UNSYNCED")
    m7h.wait_until(lambda: any("PIDCOLLAB UNSYNCED:" in line for line in oh.told(gui)[before:]), 5)
    time.sleep(0.3)
    lines = [unmarked(line) for line in oh.told(gui)[before:]]
    head = next((i for i, line in enumerate(lines) if line.startswith("PIDCOLLAB UNSYNCED:")), None)
    if head is None:
        return None, []
    rest = []
    for line in lines[head + 1:]:
        if not line.startswith("  "):
            break
        rest.append(line.strip())
    return lines[head], rest


def by_state(entries):
    out = {}
    for entry in entries or []:
        out[entry.get("state")] = out.get(entry.get("state"), 0) + 1
    return out


try:
    if not m7h.SEED_DWG.exists():
        R.check("setup", False, f"种子图不存在：{m7h.SEED_DWG}")
        raise SystemExit(1)
    dwg_a = m7h.fresh_copy("o06-PID-1101-a.dwg", m7h.SEED_DWG)
    srv = m7h.Server(3240, tag="o06-srv").start(fresh=True)
    R.check("setup", srv.wait_health(), f"服务器 /health = {srv.health()}")
    u1 = started("o06-u1", 3210, dwg_a, user="u1")
    resp = placed = None
    known = set(u1.tagged())
    if u1.wait_status(lambda s: s.get("connected") and s.get("session_id"), 30):
        resp = oh.set_prop(u1, PRESSURE, "1.0MPa")
        placed = u1.run("PIDINSERT VALVE 400,200")
    at2 = u1.wait_status(lambda s: s.get("base_revision") == 2 and s.get("queued_ops") == 0, 15)
    fresh = sorted(set(u1.tagged()) - known)
    valve = fresh[0] if fresh else ""
    original = oh.value(u1, TEMPERATURE)
    R.check("setup", resp is not None and yh.ok(resp) and placed is not None and yh.ok(placed) and at2 is not None
            and bool(valve),
            f"u1 加入，在线改设计压力为 1.0MPa、放一个阀门 {valve!r}：到修订 2，队列为空")

    # ── OS-18：面板上待同步的一行 ──────────────────────────────────────────────────────────────
    online = panel(u1)
    status = u1.status()
    R.check("OS-18", PANEL in online and SYNC not in online and status.get("connected") and status.get("queued_ops") == 0,
            f"在线、队列空：面板{'在' if PANEL in online else '不在'}，待同步那一行{'在' if SYNC in online else '不在'}"
            f"（connected={status.get('connected')} queued_ops={status.get('queued_ops')}）")
    srv.kill()
    R.check("setup", oh.away(u1), "杀掉服务器；u1 未连接")
    r1 = oh.set_prop(u1, TEMPERATURE, "150℃")
    r2 = oh.set_prop(u1, NOTE, "离线一", obj=valve)
    waiting = u1.wait_status(lambda s: s.get("connected") is False and s.get("queued_ops") == 2, 10)
    shot = m7h.SHOTS / "offline-o06-sync.png"
    away = panel(u1, shot)
    status = waiting or u1.status()
    R.dump("o06-offline-panel.json", {"status": status, "bounds": away})
    R.check("OS-18", yh.ok(r1) and yh.ok(r2) and waiting is not None and SYNC in away and PANEL in away,
            f"离线改两步：connected={status.get('connected')} queued_ops={status.get('queued_ops')}，"
            f"面板上待同步那一行{'在' if SYNC in away else '不在'}（{away.get(SYNC)}）；截图 shots/{shot.name}"
            f"{'' if shot.exists() else '（没存下）'}")

    # ── OS-17：只有排队的 ──────────────────────────────────────────────────────────────────────
    head, rest = listing(u1)
    R.check("OS-17", head == "PIDCOLLAB UNSYNCED: 排队 2 条，被拒 0 条，没排上队 0 条"
            and [line.split(" ", 1)[0] for line in rest] == ["排队", "排队"],
            f"PIDCOLLAB UNSYNCED：{head!r}，下面 {rest}")

    # ── OS-18：队列不空时退出问一次 ────────────────────────────────────────────────────
    saved = oh.save(u1)
    R.check("setup", saved[0], f"u1 保存 A（{saved[1]}）")
    first = quit_asked(u1)
    cancelled = answer(u1, "dialog_cancel") if first else (0, None)
    time.sleep(3)
    stayed = u1.running()
    after_cancel = (modal(u1), u1.status().get("queued_ops")) if stayed else (None, None)
    R.check("OS-18", first == ASKED and cancelled[0] == 200 and stayed and after_cancel == (None, 2),
            f"队列两步时 QUIT：问了 {first!r}；答「取消」（dialog_cancel -> {cancelled[0]}）后 3 秒程序{'还在' if stayed else '退了'}，"
            f"这个对话框{'还开着' if after_cancel[0] else '关了'}，队列 {after_cancel[1]} 步")
    second = quit_asked(u1)
    agreed = answer(u1, "dialog_ok") if second else (0, None)
    gone = u1.wait_exit(20)
    R.check("OS-18", second == ASKED and agreed[0] == 200 and gone,
            f"再 QUIT：又问了 {second!r}；答「退出」（dialog_ok -> {agreed[0]}）后{'退出了，没有再问' if gone else ' 20 秒内没有退出'}")
    if not gone:
        u1.kill()

    # ── OS-18：提交完数字归零；队列空时退出不问 ─────────────────────────────────────────────
    srv.start(fresh=False)
    R.check("setup", srv.wait_health(), "服务器用同一个库重新起来")
    ws, welcome = oh.ws_join(srv, "o06-u2", "u2")
    granted = claim(ws, oh.OBJ) if ws.session_id else None
    R.check("setup", (granted or {}).get("type") == "claim_granted",
            f"WS 会话（u2）加入并占住 {oh.OBJ}：{(granted or {}).get('type')}")
    R.check("setup", oh.reopen(u1), "u1 重开 A")
    taken = oh.joined(u1, 60, also=lambda s: s.get("unsynced_count") == 1)
    status = taken or u1.status()
    codes = [r.get("code") for r in status.get("rejections") or []]
    shot = m7h.SHOTS / "offline-o06-zero.png"
    zero = panel(u1, shot)
    R.dump("o06-zero.json", {"status": status, "bounds": zero})
    R.check("OS-18", taken is not None and codes == ["CLAIM_HELD"] and oh.server_value(srv, NOTE, valve) == "离线一"
            and oh.value(u1, TEMPERATURE) == original and PANEL in zero and SYNC not in zero,
            f"服务器回来后：设计温度那步被拒（拒绝记录的码 {codes}）、撤回成 {oh.value(u1, TEMPERATURE)!r}，"
            f"备注那步到了服务器（{oh.server_value(srv, NOTE, valve)!r}）；queued_ops={status.get('queued_ops')} "
            f"unsynced_count={status.get('unsynced_count')}，面板上待同步那一行{'还在' if SYNC in zero else '没了'}；"
            f"截图 shots/{shot.name}")
    saved = oh.save(u1)
    asked = quit_asked(u1, 4)
    gone = u1.wait_exit(10)
    R.check("OS-18", saved[0] and asked is None and gone,
            f"队列为空时保存（{saved[1]}）后 QUIT：{'没有问' if asked is None else '问了 ' + repr(asked)}，"
            f"{'退出了' if gone else '没有退出'}")
    if not gone:
        answer(u1, "dialog_ok")
        if not u1.wait_exit(20):
            u1.kill()
    ws.close()
    srv.kill()

    # ── OS-17：排队的、被拒的、没排上队的 ──────────────────────────────────────────────────
    R.check("setup", oh.reopen(u1), "杀掉服务器，u1 重开 A")
    resp = oh.set_prop(u1, TEMPERATURE, "80℃", obj=valve)
    R.check("setup", yh.ok(resp) and u1.wait_status(lambda s: s.get("queued_ops") == 1, 10) is not None,
            f"u1 离线改 {valve} 的设计温度为 80℃，排队一步")
    finish(u1, "A：离线改了阀门的设计温度")
    dwg_b = m7h.fresh_copy("o06-b.dwg", dwg_a)
    plain = started("o06-b-plain", 3211, dwg_b, collab=False)
    resp = oh.set_prop(plain, MATERIAL, "碳钢", obj=valve)
    R.check("setup", yh.ok(resp) and bool(oh.told_with(plain, "未同步清单")),
            f"A 复制成 B，不连协同的实例在 B 上改 {valve} 的材质为碳钢，命令行说记进了未同步清单")
    finish(plain, "B：不连协同改了材质")
    b = started("o06-b", 3212, dwg_b, user="u1")
    loaded = b.wait_status(lambda s: s.get("unsynced_count") == 2, 20)
    status = loaded or b.status()
    head, rest = listing(b)
    exported = oh.export(b, m7h.WORK / "o06-b-unsynced.json")
    entries = (exported or {}).get("entries") or []
    counts = by_state(entries)
    R.dump("o06-b-listing.json", {"status": status, "head": head, "rest": rest, "export": exported})
    R.check("OS-17", head == "PIDCOLLAB UNSYNCED: 排队 1 条，被拒 1 条，没排上队 1 条"
            and sorted(line.split(" ", 1)[0] for line in rest) == sorted(["排队", "被拒", "没排上队"]),
            f"u1 打开 B，PIDCOLLAB UNSYNCED：{head!r}，下面 {rest}")
    R.check("OS-17", counts == {"queued": 1, "refused": 1, "unqueued": 1} and len(entries) == 3
            and status.get("queued_ops") == counts.get("queued")
            and status.get("unsynced_count") == counts.get("refused", 0) + counts.get("unqueued", 0),
            f"PIDCOLLAB EXPORT 导出 {len(entries)} 条，按状态 {counts}；状态接口 queued_ops={status.get('queued_ops')} "
            f"unsynced_count={status.get('unsynced_count')}（align={status.get('align')}）")
    finish(b, "B：在会话里保存")
    on_file = oh.read_file("o06-b-file", 3213, dwg_b)
    listed = on_file.get(oh.UNSYNCED) or []
    queued_id = next((e.get("op_id") for e in entries if e.get("state") == "queued"), None)
    refused_id = next((e.get("op_id") for e in entries if e.get("state") == "refused"), None)
    R.dump("o06-b-file.json", on_file)
    R.check("OS-17", [op for op, _user in oh.file_queue(on_file)] == [queued_id] and by_state(listed) == {"refused": 1, "unqueued": 1}
            and next((e.get("op_id") for e in listed if e.get("state") == "refused"), None) == refused_id,
            f"保存后不连协同的实例读 B：队列 {oh.file_queue(on_file)}（导出里排队的是 {queued_id}），"
            f"清单按状态 {by_state(listed)}，被拒那条{'与导出的同号' if refused_id else '导出里没有'}")
except SystemExit:
    raise
except Exception as error:  # noqa: BLE001
    import traceback
    R.check("script", False, f"脚本异常: {error!r}\n{traceback.format_exc()}")
finally:
    m7h.cleanup()
    ok = R.summary()
sys.exit(0 if ok else 1)
