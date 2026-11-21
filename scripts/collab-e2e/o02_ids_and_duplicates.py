# -*- coding: utf-8 -*-
"""OS-05、OS-06、OS-07（SPEC-collab-offline-sync.md §7、§8）：修改编号与重复提交。

u1（3210）、u2（3211）各开一份种子图（一个容器 P-1001），连同一个服务器（3240）；
在线时 u1 改一次设计压力（修订 1），两边都保存，图里有了同步记录。

OS-05：杀掉服务器。u1 离线改一步（设计压力 1.6MPa），保存、退出；服务器仍不开，重开
再改一步（材质 不锈钢），保存、退出。两次离线运行排的两步编号不同、运行标识不同；服务器
起来后两步都提交，各是一个新修订，没有一条被吞掉。
OS-06：同一段时间里 u2 也离线改一步（设计温度 120℃），保存、退出。服务器起来后 u1
先重开，两步提交为修订 2、3；u2 后重开，补齐这两步再提交自己的（修订 4）。各自的修改
都在对方图上，两端的图逐项相同。
OS-07：再杀掉服务器，u1 离线改两步、保存、退出，把这份图复制一份（第二台机器上的
同一份图，带同样的队列）。服务器起来后 u1 重开、提交（修订 5、6）；然后另一个实例
（用户仍是 u1，自己的 APPDATA）打开那份副本。房间里每条修改只生效一次，三份图相同。
最后用一个 WS 会话把日志里已有的一个编号带另一个值再发一次：服务器不再裁决，只回
当初那条 op_applied（修订号和变更取日志里那一行），不广播。

证据：logs/o02-*.json、logs/gui-o02-*.err.log、logs/o02-srv.out.log、logs/ws-o02-dup.jsonl。
"""
import sys
import time

import m7h
import oh
import yh

R = m7h.Results("o02_ids_and_duplicates")
PRESSURE, TEMPERATURE, MATERIAL = "设计压力", "设计温度", "材质"


def changes(exported):
    """(field, value) of each queued entry of an export, in its order."""
    out = []
    for entry in (exported or {}).get("entries") or []:
        if entry.get("state") == "queued":
            first = (entry.get("changes") or [{}])[0]
            out.append((first.get("field"), first.get("value")))
    return out


def answered(gui, op_ids):
    """The revisions of the op_applied frames `gui` got since its last start for `op_ids`, in their order."""
    return [f.get("revision") for f in m7h.frames(gui.err_text(), "<-", "op_applied") if f.get("op_id") in op_ids]


try:
    if not m7h.SEED_DWG.exists():
        R.check("setup", False, f"种子图不存在：{m7h.SEED_DWG}")
        raise SystemExit(1)
    dwg_a = m7h.fresh_copy("o02-PID-1101-a.dwg", m7h.SEED_DWG)
    dwg_b = m7h.fresh_copy("o02-PID-1101-b.dwg", m7h.SEED_DWG)
    srv = m7h.Server(3240, tag="o02-srv").start(fresh=True)
    R.check("setup", srv.wait_health(), f"服务器 /health = {srv.health()}")
    u1 = m7h.Gui("o02-u1", 3210, dwg=dwg_a, user="u1").start()
    u2 = m7h.Gui("o02-u2", 3211, dwg=dwg_b, user="u2").start()
    R.check("setup", u1.wait_ready(180) is not None and u2.wait_ready(180) is not None, "两个 GUI 就绪")
    s1 = u1.wait_status(lambda s: s.get("connected") and s.get("session_id"), 30)
    s2 = u2.wait_status(lambda s: s.get("connected") and s.get("session_id"), 30)
    R.check("setup", s1 is not None and s2 is not None, "u1、u2 均已连接")
    resp = oh.set_prop(u1, PRESSURE, "1.0MPa")
    at1 = all(g.wait_status(lambda s: s.get("base_revision") == 1 and s.get("queued_ops") == 0, 15) is not None
              for g in (u1, u2))
    saved = oh.save(u1)[0] and oh.save(u2)[0]
    R.check("setup", yh.ok(resp) and at1 and saved, "在线时 u1 改设计压力为 1.0MPa，两边到修订 1 并各自保存")

    # ── OS-05 / OS-06：离线的两次运行，另一个人同时也离线 ───────────────────────
    srv.kill()
    R.check("setup", oh.away(u1) and oh.away(u2), "服务器被杀掉后两边都报告未连接")
    resp = oh.set_prop(u1, PRESSURE, "1.6MPa")
    saved = oh.save(u1)
    first = oh.export(u1, m7h.WORK / "o02-u1-run1.json")
    gone, how = oh.quit_gui(u1)
    R.check("setup", yh.ok(resp) and saved[0] and len(oh.queued_ids(first)) == 1 and gone,
            f"u1 第一次离线运行：改设计压力为 1.6MPa，保存，队列 {oh.queued_ids(first)}，退出（{how}）")
    resp = oh.set_prop(u2, TEMPERATURE, "120℃")
    saved = oh.save(u2)
    theirs = oh.export(u2, m7h.WORK / "o02-u2-run1.json")
    gone, how = oh.quit_gui(u2)
    R.check("setup", yh.ok(resp) and saved[0] and len(oh.queued_ids(theirs)) == 1 and gone,
            f"u2 同时也离线：改设计温度为 120℃，保存，队列 {oh.queued_ids(theirs)}，退出（{how}）")
    R.check("setup", oh.reopen(u1), "服务器仍不开，u1 重开")
    resp = oh.set_prop(u1, MATERIAL, "不锈钢")
    saved = oh.save(u1)
    second = oh.export(u1, m7h.WORK / "o02-u1-run2.json")
    gone, how = oh.quit_gui(u1)
    R.dump("o02-exports.json", {"u1_run1": first, "u1_run2": second, "u2_run1": theirs})
    R.check("setup", yh.ok(resp) and saved[0] and gone,
            f"u1 第二次离线运行：改材质为不锈钢，保存，退出（{how}）")
    ids = oh.queued_ids(second)
    runs = [oh.run_of(i) for i in ids]
    R.check("OS-05", len(ids) == 2 and ids[:1] == oh.queued_ids(first) and len(set(ids)) == 2
            and all(runs) and runs[0] != runs[1] and changes(second) == [(PRESSURE, "1.6MPa"), (MATERIAL, "不锈钢")],
            f"两次离线运行排的两步：{changes(second)}，编号 {ids}（运行标识 {runs}），"
            f"第一步是第一次运行排的 {oh.queued_ids(first)}")

    # ── 先后回来 ──────────────────────────────────────────────────────────────────
    srv.start(fresh=False)
    R.check("setup", srv.wait_health(), "服务器用同一个库重新起来")
    R.check("setup", oh.reopen(u1), "u1 先重开")
    b1 = oh.joined(u1, 90, also=lambda s: s.get("base_revision") == 3)
    rows = oh.ops_rows(srv)
    R.dump("o02-ops-after-u1.json", rows)
    R.check("OS-05", b1 is not None and [r["op_id"] for r in rows] [1:] == ids and all(r["user_id"] == "u1" for r in rows)
            and [f.get("op_id") for f in oh.sent_ops(u1)] == ids and answered(u1, ids) == [2, 3]
            and oh.server_value(srv, PRESSURE) == "1.6MPa" and oh.server_value(srv, MATERIAL) == "不锈钢",
            f"u1 连上后两步都提交：发出 {[f.get('op_id') for f in oh.sent_ops(u1)]}，答复的修订 {answered(u1, ids)}；"
            f"服务器日志 {[(r['revision'], r['op_id'], r['user_id']) for r in rows]}；服务器读回设计压力 "
            f"{oh.server_value(srv, PRESSURE)}、材质 {oh.server_value(srv, MATERIAL)}")
    R.check("setup", oh.reopen(u2), "u2 后重开")
    b2 = oh.joined(u2, 90, also=lambda s: s.get("base_revision") == 4)
    oh.joined(u1, 30, also=lambda s: s.get("base_revision") == 4)
    rows = oh.ops_rows(srv)
    R.dump("o02-ops-after-u2.json", rows)
    mine = oh.queued_ids(theirs)
    every = ids + mine
    seen = {"u1": (oh.value(u1, PRESSURE), oh.value(u1, MATERIAL), oh.value(u1, TEMPERATURE)),
            "u2": (oh.value(u2, PRESSURE), oh.value(u2, MATERIAL), oh.value(u2, TEMPERATURE))}
    R.check("OS-06", b2 is not None and len(rows) == 4 and rows[-1]["op_id"] == (mine or [None])[0]
            and rows[-1]["user_id"] == "u2" and len(set(every)) == 3 and len({oh.run_of(i) for i in every}) == 3
            and seen["u1"] == seen["u2"] == ("1.6MPa", "不锈钢", "120℃"),
            f"u2 补齐 u1 的两步后提交自己的：服务器日志 {[(r['revision'], r['op_id'], r['user_id']) for r in rows]}；"
            f"设计压力、材质、设计温度 u1 {seen['u1']}，u2 {seen['u2']}；三个编号 {every}")
    a1, a2, same = yh.wait_same(u1, u2, 20)
    R.dump("o02-drawings-after-both.json", {"u1": a1["authored"], "u2": a2["authored"]})
    fine, why = yh.not_diverged(u1, u2)
    R.check("OS-06", same and fine and not u1.status().get("rejections") and not u2.status().get("rejections")
            and (oh.server_value(srv, PRESSURE), oh.server_value(srv, MATERIAL), oh.server_value(srv, TEMPERATURE))
            == ("1.6MPa", "不锈钢", "120℃"),
            f"两端的图逐项相同（位号、属性、拓扑）：{same}；没有被拒的，没有不一致：{why}")

    # ── OS-07：同一份带队列的图在两台机器上先后打开 ──────────────────────────────
    srv.kill()
    R.check("setup", oh.away(u1) and oh.away(u2), "第三次杀掉服务器")
    answers = [oh.set_prop(u1, PRESSURE, "4.0MPa"), oh.set_prop(u1, MATERIAL, "碳钢")]
    saved = oh.save(u1)
    third = oh.export(u1, m7h.WORK / "o02-u1-run3.json")
    gone, how = oh.quit_gui(u1)
    twice = oh.queued_ids(third)
    dwg_copy = m7h.fresh_copy("o02-PID-1101-a-copy.dwg", dwg_a)
    R.dump("o02-copied-queue.json", third)
    R.check("setup", all(yh.ok(a) for a in answers) and saved[0] and len(twice) == 2 and gone,
            f"u1 离线改两步（{changes(third)}，编号 {twice}），保存、退出（{how}），图复制为 {dwg_copy.name}")
    srv.start(fresh=False)
    R.check("setup", srv.wait_health(), "服务器又用同一个库起来")
    oh.joined(u2, 90)
    R.check("setup", oh.reopen(u1), "第一台机器：u1 重开原来那份图")
    b1 = oh.joined(u1, 90, also=lambda s: s.get("base_revision") == 6)
    R.check("setup", b1 is not None, f"u1 提交了两步，到修订 {(b1 or u1.status()).get('base_revision')}")
    m2 = m7h.Gui("o02-m2", 3212, dwg=dwg_copy, user="u1").start()
    R.check("setup", m2.wait_ready(180) is not None, "第二台机器：另一个实例（用户 u1）打开副本")
    b2 = oh.joined(m2, 90, also=lambda s: s.get("base_revision") == 6)
    time.sleep(2)
    rows = oh.ops_rows(srv)
    R.dump("o02-ops-after-copy.json", rows)
    logged = [r["op_id"] for r in rows]
    m2_sent = [f.get("op_id") for f in oh.sent_ops(m2)]
    a1, a3, same_copy = yh.wait_same(u1, m2, 20)
    a1, a2, same_peer = yh.wait_same(u1, u2, 20)
    fine, why = yh.not_diverged(u1, u2, m2)
    R.check("OS-07", b2 is not None and len(rows) == 6 and [logged.count(i) for i in twice] == [1, 1]
            and [r["revision"] for r in rows if r["op_id"] in twice] == [5, 6] and same_copy and same_peer and fine
            and (oh.value(m2, PRESSURE), oh.value(m2, MATERIAL)) == ("4.0MPa", "碳钢"),
            f"副本连上后队列清空、到修订 {(b2 or m2.status()).get('base_revision')}；服务器日志共 {len(rows)} 行，"
            f"两个编号各出现 {[logged.count(i) for i in twice]} 次；副本发出的 op {m2_sent}，收到这两个编号的 "
            f"op_applied 修订 {answered(m2, twice)}；三份图相同 {same_copy and same_peer}，没有不一致：{why}")
    heard = len([f for f in m7h.frames(u2.err_text(), "<-", "op_applied") if f.get("op_id") == twice[0]])
    ws, welcome = oh.ws_join(srv, "o02-dup", "u1")
    answer = oh.ws_write(ws, (welcome or {}).get("base_revision"), twice[0], PRESSURE, "9.9MPa")
    time.sleep(1.5)
    after = oh.ops_rows(srv)
    heard_after = len([f for f in m7h.frames(u2.err_text(), "<-", "op_applied") if f.get("op_id") == twice[0]])
    echoed = [c.get("value") for c in (answer or {}).get("changes") or []]
    R.dump("o02-duplicate-answer.json", {"welcome": welcome, "answer": answer, "ops": after})
    R.check("OS-07", (answer or {}).get("type") == "op_applied" and answer.get("revision") == 5 and echoed == ["4.0MPa"]
            and len(after) == 6 and heard_after == heard and oh.server_value(srv, PRESSURE) == "4.0MPa",
            f"已在日志里的编号 {twice[0]} 带 9.9MPa 再发一次：答复 {(answer or {}).get('type')} 修订 "
            f"{(answer or {}).get('revision')}，变更里的值 {echoed}；服务器日志仍是 {len(after)} 行，设计压力仍是 "
            f"{oh.server_value(srv, PRESSURE)}；u2 收到这个编号的 op_applied {heard} → {heard_after} 条")
    ws.close()
    for name, g in (("u1", u1), ("m2", m2)):
        st, _resp, shot = g.capture(m7h.SHOTS / f"offline-o02-{name}.png")
        R.note(f"{name} 截图 shots/offline-o02-{name}.png：HTTP {st}，文件{'在' if shot else '不在'}")
except SystemExit:
    raise
except Exception as error:  # noqa: BLE001
    import traceback
    R.check("script", False, f"脚本异常: {error!r}\n{traceback.format_exc()}")
finally:
    m7h.cleanup()
    ok = R.summary()
sys.exit(0 if ok else 1)
