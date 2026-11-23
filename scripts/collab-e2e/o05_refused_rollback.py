# -*- coding: utf-8 -*-
"""OS-14、OS-15（SPEC-collab-offline-sync.md §5.3、§5.4、§7、§8，§9 L5）：重开后被拒的一步在本图撤回，让位的一步跟房间走。

准备：服务器（3240，新库），u1（3210）打开种子图加入，在线改 P-1001 的设计压力为 1.0MPa，放四个阀门
VA、VB、VC、VD，画管线 L-1001 和它上面的管路 L-1001-01，再画管线 L-1002。一个 WS 会话（u2）在 L-1002
上加一条从 VC 出发的管路：服务器只按管路的 from/to 认引用，客户端画管路时不带这两个字段，所以这条由
WS 会话来加。u1 存成 A、退出；A 复制成 X、P、Y、L5 和 u2 用的一份。杀掉服务器，前四份各自离线改、
保存、退出：

- X：P-1001 的设计温度（之后 u2 占着它）、删 VC（被 L-1002 引用）、在 L-1001 上再画一条管路
  L-1001-02（之后 u2 在 L-1001 上放阀门）、VB 的备注（无关的一步）。
- Y：P-1001 的设计压力（之后 u2 改成别的值）、VA 的备注（之后 u2 删掉 VA）、VB 的设计压力（无关）。
- P：VB 的材质、P-1001 的备注（之后图纸状态改成只读）。
- L5：删 VD（之后 u2 改了 VD 的设计压力）。

服务器用同一个库起来，u2（3215）打开它那份加入：在 L-1001-01 上放闸阀、P-1001 的设计压力改成
2.5MPa、删 VA、VD 的设计压力改成 0.6MPa、占用 P-1001。

OS-14（占用、删除仍被引用的对象、别人动过同一条线）：X 重开。前三步各以 CLAIM_HELD、
OBJECT_REFERENCED、STALE_BASE_REVISION 被拒，在 X 里撤回（设计温度回到原值、VC 回来、L-1001-02
没了）；之后无关的 VB 备注照常提交；X 与 u2 的图一致、都不标不一致；拒绝记录和未同步清单里的
被拒条目（已撤回）各三条，op_id 一一对上；服务器只多了一条 op。

OS-15（让位）：u2 放开占用后 Y 重开。设计压力那步落后让位（OP_SUPERSEDED），VA 那步因对象已删让位
（OBJECT_DELETED），VB 那步提交；Y 的图与 u2 的一致（设计压力是 2.5MPa、VA 没了），不标不一致。

L5（§9 L5，只记录不评判）：L5 重开，记下删 VD 这一步的结果、服务器上 VD 还在不在、u2 和 L5 的
图里 VD 还在不在。

OS-14（没有写权限）：图纸状态改成只读后 P 重开。写权限按会话判，不按对象分：P 的两步都以
PERM_DENIED_STATE 被拒并撤回，P 与 u2 一致，拒绝记录与清单各两条，服务器没多 op。（「之后无关的步骤
照常提交」在 X 里验：没有写权限时这份里没有能提交的一步。）

REQ-COLL-004（只读切回）：最后项目管理员 u4 把图纸从只读切回协同编辑（切换图纸状态只过角色和 PBS 两级，
不受当前状态限制），一个新的 WS 会话（u2）再改 P-1001 的备注：提交成功，服务器上是新值。

被拒的是从文件里恢复的步骤，没有撤销栈可退，程序按反向修改撤回，拒绝记录里说的是「已按反向修改撤回」。

证据：logs/o05-*.json、logs/gui-o05-*.err.log、logs/o05-srv.out.log、logs/ws-o05-*.jsonl。
"""
import sys

import m7h
import oh
import yh

R = m7h.Results("o05_refused_rollback")
PRESSURE, TEMPERATURE, NOTE, MATERIAL = "设计压力", "设计温度", "备注", "材质"
OBJ = oh.OBJ
NEW_RUN = "L-1001-02"


def started(name, port, dwg, **more):
    gui = m7h.Gui(name, port, dwg=dwg, **more).start()
    R.check("setup", gui.wait_ready(180) is not None, f"{name} 打开 {dwg.name}")
    return gui


def finish(gui, what):
    saved = oh.save(gui)
    gone, how = oh.quit_gui(gui)
    R.check("setup", saved[0] and gone, f"{what}：保存（{saved[1]}）、退出（{how}）")


def ws_op(ws, base, op_id, payload, seconds=10):
    """Send one op; the answer that names it (op_applied, error, conflict), or None."""
    ws.send({"type": "op", "session_id": ws.session_id, "base_revision": base, "op_id": op_id, "payload": payload})
    return ws.wait_for(lambda m: m.get("type") in ("op_applied", "error", "conflict") and m.get("op_id") == op_id,
                       seconds)


def erase_tag(gui, tag):
    handle = gui.tagged().get(tag)
    return yh.erase(gui, [handle]) if handle else (0, {"ok": False, "error": f"没有 {tag}"})


def runs_of(gui):
    """The tags of the pipe runs of `gui`'s drawing whose body is there."""
    table = yh.effective(yh.topology(gui), yh.entities(gui))
    return sorted(run["tag"] for run in table["pipe_runs"])


def server_runs(srv):
    st, body = srv.http("GET", f"/drawing/{oh.DRAWING}/topology")
    body = body or {}
    runs = body.get("pipe_runs") or (body.get("topology") or {}).get("pipe_runs") or []
    return {run.get("tag"): run for run in runs}


def refusals(status):
    """(code, outcome, op_id) of every rejection record the status lists, in order."""
    return [(r.get("code"), r.get("outcome"), r.get("op_id")) for r in status.get("rejections") or []]


def rolled(outcome):
    """Whether a rejection record says the restored step was taken back by its inverse."""
    return str(outcome or "").startswith("已按反向修改撤回")


def refused_entries(exported):
    return [e for e in (exported or {}).get("entries") or [] if e.get("state") == "refused"]


def error_frames(gui):
    return [(f.get("code"), f.get("rejected")) for f in m7h.frames(gui.err_text(), "<-", "error") if f.get("op_id")]


def come_back(gui, name, refused, seconds=90):
    """Reopen `gui` with the server up; its status once the queue is answered and `refused` rejections are listed."""
    R.check("setup", oh.reopen(gui), f"{name} 重开")
    back = oh.joined(gui, seconds, also=lambda s: len(s.get("rejections") or []) >= refused)
    return back, back or gui.status()


try:
    if not m7h.SEED_DWG.exists():
        R.check("setup", False, f"种子图不存在：{m7h.SEED_DWG}")
        raise SystemExit(1)
    dwg_a = m7h.fresh_copy("o05-PID-1101-a.dwg", m7h.SEED_DWG)
    srv = m7h.Server(3240, tag="o05-srv").start(fresh=True)
    R.check("setup", srv.wait_health(), f"服务器 /health = {srv.health()}")
    u1 = started("o05-u1", 3210, dwg_a, user="u1")
    known = set(u1.tagged())
    steps = []
    if u1.wait_status(lambda s: s.get("connected") and s.get("session_id"), 30):
        steps.append(oh.set_prop(u1, PRESSURE, "1.0MPa"))
        for x in (400, 500, 600, 700):
            steps.append(u1.run(f"PIDINSERT VALVE {x},200"))
        steps.append(yh.draw(u1, "PIDPIPE", [(0, 0), (200, 0)]))
        steps.append(yh.draw(u1, "PIDPIPE run", [(0, 0), (200, 0)]))
        steps.append(yh.draw(u1, "PIDPIPE", [(0, -150), (200, -150)]))
    settled = u1.wait_status(lambda s: s.get("base_revision") == len(steps) and s.get("queued_ops") == 0, 20)
    valves = sorted(set(u1.tagged()) - known)
    va, vb, vc, vd = (valves + [""] * 4)[:4]
    lines = sorted(line["tag"] for line in yh.effective(yh.topology(u1), yh.entities(u1))["pipe_lines"])
    R.check("setup", len(steps) == 8 and all(yh.ok(r) for r in steps) and settled is not None and len(valves) == 4
            and lines == ["L-1001", "L-1002"] and runs_of(u1) == ["L-1001-01"],
            f"u1 加入，在线改设计压力、放四个阀门 {valves}、画管线 {lines} 和管路 {runs_of(u1)}："
            f"到修订 {(settled or {}).get('base_revision')}，队列为空")
    base = (settled or {}).get("base_revision") or len(steps)

    ws, welcome = oh.ws_join(srv, "o05-ws", "u2")
    said = ws_op(ws, base, "o05-ws-run-1", {"kind": "add_pipe_run", "line_tag": "L-1002",
                                             "points": [[0, -150], [200, -150]], "from": vc}) if ws.session_id else None
    ws.close()
    known_runs = server_runs(srv)
    followed = u1.wait_status(lambda s: s.get("base_revision") == base + 1 and s.get("queued_ops") == 0, 15)
    has_run = m7h.wait_until(lambda: "L-1002-01" in runs_of(u1), 10)
    R.dump("o05-setup.json", {"said": said, "server_runs": known_runs, "valves": valves})
    R.check("setup", (said or {}).get("type") == "op_applied" and (known_runs.get("L-1002-01") or {}).get("from") == vc
            and followed is not None and bool(has_run),
            f"WS 会话（u2）在 L-1002 上加一条从 {vc} 出发的管路：{(said or {}).get('type')}；服务器记的管路 "
            f"{ {tag: (run.get('line_tag'), run.get('from')) for tag, run in known_runs.items()} }；u1 到修订 "
            f"{(followed or {}).get('base_revision')}，图里有了 L-1002-01")
    original = {"temperature": oh.value(u1, TEMPERATURE), "note": oh.value(u1, NOTE),
                "material": oh.value(u1, MATERIAL, vb)}
    finish(u1, "u1 存成 A")
    copies = {name: m7h.fresh_copy(f"o05-{name}.dwg", dwg_a) for name in ("x", "p", "y", "l5", "u2")}
    srv.kill()

    # ── 各份离线改 ───────────────────────────────────────────────────────────────────────────────
    x = started("o05-x", 3211, copies["x"], user="u1")
    done = [oh.set_prop(x, TEMPERATURE, "150℃"), erase_tag(x, vc),
            yh.draw(x, "PIDPIPE run L-1001", [(0, 40), (200, 40)]), oh.set_prop(x, NOTE, "离线X", obj=vb)]
    queued = x.wait_status(lambda s: s.get("connected") is False and s.get("queued_ops") == 4, 10)
    R.check("setup", all(yh.ok(r) for r in done) and queued is not None and NEW_RUN in runs_of(x) and vc not in x.tagged(),
            f"X 离线：改 {OBJ} 的设计温度、删 {vc}、在 L-1001 上画 {NEW_RUN}、改 {vb} 的备注，"
            f"排队 {(queued or x.status()).get('queued_ops')} 步")
    finish(x, "X")
    y = started("o05-y", 3213, copies["y"], user="u1")
    done = [oh.set_prop(y, PRESSURE, "1.6MPa"), oh.set_prop(y, NOTE, "离线Y", obj=va),
            oh.set_prop(y, PRESSURE, "0.8MPa", obj=vb)]
    queued = y.wait_status(lambda s: s.get("connected") is False and s.get("queued_ops") == 3, 10)
    R.check("setup", all(yh.ok(r) for r in done) and queued is not None,
            f"Y 离线：改 {OBJ} 的设计压力、{va} 的备注、{vb} 的设计压力，排队 {(queued or y.status()).get('queued_ops')} 步")
    finish(y, "Y")
    p = started("o05-p", 3212, copies["p"], user="u1")
    done = [oh.set_prop(p, MATERIAL, "不锈钢", obj=vb), oh.set_prop(p, NOTE, "离线P")]
    queued = p.wait_status(lambda s: s.get("connected") is False and s.get("queued_ops") == 2, 10)
    R.check("setup", all(yh.ok(r) for r in done) and queued is not None,
            f"P 离线：改 {vb} 的材质、{OBJ} 的备注，排队 {(queued or p.status()).get('queued_ops')} 步")
    finish(p, "P")
    l5 = started("o05-l5", 3214, copies["l5"], user="u1")
    done = erase_tag(l5, vd)
    queued = l5.wait_status(lambda s: s.get("connected") is False and s.get("queued_ops") == 1, 10)
    R.check("setup", yh.ok(done) and queued is not None and vd not in l5.tagged(), f"L5 离线：删 {vd}，排队一步")
    finish(l5, "L5")

    # ── 这期间别人做的 ──────────────────────────────────────────────────────────────────────────
    srv.start(fresh=False)
    R.check("setup", srv.wait_health(), "服务器用同一个库重新起来")
    u2 = started("o05-u2", 3215, copies["u2"], user="u2")
    done = []
    if u2.wait_status(lambda s: s.get("connected") and s.get("session_id"), 60):
        done = [yh.draw(u2, "PIDFITTING gate_valve", [(100, 0)]), oh.set_prop(u2, PRESSURE, "2.5MPa"),
                erase_tag(u2, va), oh.set_prop(u2, PRESSURE, "0.6MPa", obj=vd), u2.run(f"PIDCLAIM {OBJ}")]
    claimed = u2.wait_status(lambda s: s.get("base_revision") == base + 5 and s.get("queued_ops") == 0
                             and (s.get("claims") or {}).get(OBJ) == "u2", 20)
    rows = oh.ops_rows(srv)
    R.check("setup", len(done) == 5 and all(yh.ok(r) for r in done) and claimed is not None,
            f"u2 加入：在 L-1001-01 上放闸阀、{OBJ} 的设计压力改成 2.5MPa、删 {va}、{vd} 的设计压力改成 0.6MPa、"
            f"占用 {OBJ}：到修订 {(claimed or u2.status()).get('base_revision')}，ops 表 {len(rows)} 行")

    # ── OS-14：占用、删除仍被引用的对象、别人动过同一条线 ─────────────────────────────────────────
    back, status = come_back(x, "X", 3)
    told = refusals(status)
    codes = [code for code, _outcome, _op in told]
    exported = oh.export(x, m7h.WORK / "o05-x-unsynced.json")
    listed = refused_entries(exported)
    R.dump("o05-x-back.json", {"status": status, "export": exported, "errors": error_frames(x)})
    R.check("OS-14", back is not None and codes == ["CLAIM_HELD", "OBJECT_REFERENCED", "STALE_BASE_REVISION"]
            and all(rolled(outcome) for _code, outcome, _op in told),
            f"X 重开：服务器拒了三步 {codes}，拒绝记录说 {[outcome for _c, outcome, _o in told]}；"
            f"收到的 error 帧 {error_frames(x)}")
    temperature, runs_x, tags_x = oh.value(x, TEMPERATURE), runs_of(x), x.tagged()
    R.check("OS-14", temperature == original["temperature"] and vc in tags_x and NEW_RUN not in runs_x,
            f"三步在 X 里撤回：{OBJ} 的设计温度是 {temperature!r}（原值 {original['temperature']!r}），"
            f"{vc} {'回来了' if vc in tags_x else '没回来'}，图里的管路 {runs_x}")
    after = oh.ops_rows(srv)
    R.check("OS-14", oh.server_value(srv, NOTE, vb) == "离线X" and oh.value(x, NOTE, vb) == "离线X"
            and len(after) == len(rows) + 1 and after[-1]["user_id"] == "u1",
            f"之后无关的一步照常提交：服务器上 {vb} 的备注 {oh.server_value(srv, NOTE, vb)!r}，"
            f"ops 表 {len(rows)} → {len(after)} 行")
    a, b, same = yh.wait_same(x, u2, 20)
    fine, why = yh.not_diverged(x, u2)
    R.dump("o05-x-drawings.json", {"x": a["authored"], "u2": b["authored"], "x_topology": a["topology"],
                                   "u2_topology": b["topology"]})
    R.check("OS-14", same and fine and oh.value(u2, NOTE, vb) == "离线X" and vc in u2.tagged()
            and NEW_RUN not in runs_of(u2),
            f"X 与 u2 的图{'一致' if same else '不一致'}（带 PID_AUTHOR 的实体与拓扑），都不标不一致：{why}")
    R.check("OS-14", sorted((e.get("op_id"), e.get("code")) for e in listed) == sorted((op, code) for code, _o, op in told)
            and len(listed) == 3 and all(e.get("rolled_back") is True for e in listed)
            and status.get("unsynced_count") == 3 and status.get("queued_ops") == 0,
            f"拒绝记录 {len(told)} 条，未同步清单里被拒的 {len(listed)} 条"
            f"（{[(e.get('code'), e.get('rolled_back')) for e in listed]}），op_id 一一对上；"
            f"unsynced_count={status.get('unsynced_count')} queued_ops={status.get('queued_ops')}")
    finish(x, "X 收尾")
    u2.run(f"PIDRELEASE {OBJ}")
    R.check("setup", u2.wait_status(lambda s: OBJ not in (s.get("claims") or {}), 10) is not None, f"u2 放开 {OBJ}")

    # ── OS-15：让位 ─────────────────────────────────────────────────────────────────────────────────
    rows = oh.ops_rows(srv)
    back, status = come_back(y, "Y", 2)
    told = refusals(status)
    codes = [code for code, _outcome, _op in told]
    exported = oh.export(y, m7h.WORK / "o05-y-unsynced.json")
    R.dump("o05-y-back.json", {"status": status, "export": exported, "errors": error_frames(y)})
    R.check("OS-15", back is not None and codes == ["OP_SUPERSEDED", "OBJECT_DELETED"],
            f"Y 重开：两步让位 {codes}（{[outcome for _c, outcome, _o in told]}）；收到的 error 帧 {error_frames(y)}")
    a, b, same = yh.wait_same(y, u2, 20)
    fine, why = yh.not_diverged(y, u2)
    pressure, tags_y = oh.value(y, PRESSURE), y.tagged()
    R.dump("o05-y-drawings.json", {"y": a["authored"], "u2": b["authored"]})
    R.check("OS-15", same and fine and pressure == "2.5MPa" and va not in tags_y and status.get("align") != "diverged",
            f"Y 的图跟房间走：{OBJ} 的设计压力 {pressure!r}，{va} {'还在' if va in tags_y else '没了'}；"
            f"与 u2 的图{'一致' if same else '不一致'}，不一致标记 {why}，align={status.get('align')}")
    after = oh.ops_rows(srv)
    R.check("OS-15", oh.server_value(srv, PRESSURE, vb) == "0.8MPa" and oh.value(u2, PRESSURE, vb) == "0.8MPa"
            and len(after) == len(rows) + 1,
            f"同一次重连里无关的一步照常提交：服务器上 {vb} 的设计压力 {oh.server_value(srv, PRESSURE, vb)!r}，"
            f"ops 表 {len(rows)} → {len(after)} 行")
    finish(y, "Y 收尾")

    # ── L5：我离线删了对象、别人这期间改过它（只记录） ───────────────────────────────────────────────────
    before = oh.value(u2, PRESSURE, vd)
    R.check("setup", oh.reopen(l5), "L5 重开")
    status = l5.wait_status(lambda s: s.get("connected") and s.get("session_id") and s.get("catching_up") is None
                            and (s.get("queued_ops") == 0 or s.get("align") in ("diverged", "refused")), 60) \
        or l5.status()
    gone_in_u2 = m7h.wait_until(lambda: vd not in u2.tagged(), 10)
    st, objects = srv.http("GET", f"/drawing/{oh.DRAWING}/objects?object_ref={vd}")
    last = srv.sql("select revision, user_id, base_revision, changes from ops where drawing_id = ? "
                   "order by revision desc limit 1", (oh.DRAWING,))
    a, b, same = yh.wait_same(l5, u2, 10)
    seen = {"status": status, "rejections": refusals(status), "server_objects": objects, "last_op": last,
            "u2_had": before, "u2_has_it": not gone_in_u2, "l5_has_it": vd in l5.tagged(), "same": same}
    R.dump("o05-l5.json", seen)
    R.note(f"L5（§9 L5，只记录）：离线删 {vd}，这期间 u2 把它的设计压力改成了 {before!r}。L5 重开后："
           f"align={status.get('align')}（{status.get('align_code')}：{str(status.get('align_message'))[:90]}），"
           f"拒绝记录 {refusals(status)}，队列 {status.get('queued_ops')}；服务器最后一条 op "
           f"{[(r.get('revision'), r.get('user_id'), str(r.get('changes'))[:120]) for r in last]}；"
           f"服务器对象查询 {st} {str(objects)[:200]}；u2 的图里 {vd} {'没了' if gone_in_u2 else '还在'}，"
           f"L5 的图里 {'还在' if vd in l5.tagged() else '没了'}；两边的图{'一致' if same else '不一致'}")
    finish(l5, "L5 收尾")

    # ── OS-14：没有写权限 ───────────────────────────────────────────────────────────────────────
    st, body = srv.http("POST", f"/drawing/{oh.DRAWING}/state", {"user_id": "u4", "state": "只读"})
    R.check("setup", st == 200, f"图纸状态改成只读：{st} {body}")
    rows = oh.ops_rows(srv)
    back, status = come_back(p, "P", 2)
    told = refusals(status)
    codes = [code for code, _outcome, _op in told]
    exported = oh.export(p, m7h.WORK / "o05-p-unsynced.json")
    listed = refused_entries(exported)
    R.dump("o05-p-back.json", {"status": status, "export": exported, "errors": error_frames(p)})
    R.check("OS-14", back is not None and codes == ["PERM_DENIED_STATE", "PERM_DENIED_STATE"]
            and all(rolled(outcome) for _code, outcome, _op in told),
            f"只读时 P 重开：两步都被拒 {codes}（{[outcome for _c, outcome, _o in told]}）；收到的 error 帧 {error_frames(p)}")
    material, note = oh.value(p, MATERIAL, vb), oh.value(p, NOTE)
    a, b, same = yh.wait_same(p, u2, 20)
    fine, why = yh.not_diverged(p, u2)
    R.check("OS-14", material == original["material"] and note == original["note"] and same and fine
            and len(oh.ops_rows(srv)) == len(rows),
            f"两步在 P 里撤回：{vb} 的材质 {material!r}（原值 {original['material']!r}），{OBJ} 的备注 {note!r}"
            f"（原值 {original['note']!r}）；与 u2 的图{'一致' if same else '不一致'}，不一致标记 {why}；"
            f"ops 表仍是 {len(rows)} 行")
    R.check("OS-14", sorted((e.get("op_id"), e.get("code")) for e in listed) == sorted((op, code) for code, _o, op in told)
            and len(listed) == 2 and all(e.get("rolled_back") is True for e in listed)
            and status.get("unsynced_count") == 2,
            f"拒绝记录 {len(told)} 条，未同步清单里被拒的 {len(listed)} 条"
            f"（{[(e.get('code'), e.get('rolled_back')) for e in listed]}），op_id 一一对上；unsynced_count={status.get('unsynced_count')}")

    # ── REQ-COLL-004：只读切回协同编辑 ──────────────────────────────────────────────────────────────
    st, body = srv.http("POST", f"/drawing/{oh.DRAWING}/state", {"user_id": "u4", "state": "协同编辑"})
    ws, welcome = oh.ws_join(srv, "o05-u2-after", "u2")
    answer = (oh.ws_write(ws, (welcome or {}).get("base_revision") or 0, "op-o05-after-readonly", NOTE, "切回之后")
              if ws.session_id else None)
    now = oh.server_value(srv, NOTE)
    R.check("REQ-COLL-004", st == 200 and (answer or {}).get("type") == "op_applied" and now == "切回之后",
            f"项目管理员 u4 把图纸从只读切回协同编辑：{st} {body}；之后 u2 改 {OBJ} 的备注："
            f"{(answer or {}).get('type')} {(answer or {}).get('code') or ''}，服务器上是 {now!r}")
    ws.close()
    finish(p, "P 收尾")
except SystemExit:
    raise
except Exception as error:  # noqa: BLE001
    import traceback
    R.check("script", False, f"脚本异常: {error!r}\n{traceback.format_exc()}")
finally:
    m7h.cleanup()
    ok = R.summary()
sys.exit(0 if ok else 1)
