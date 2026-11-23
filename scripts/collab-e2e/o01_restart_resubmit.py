# -*- coding: utf-8 -*-
"""OS-03、OS-04（SPEC-collab-offline-sync.md §7、§8）：离线改的步骤跨过程序重启提交。

u1（3210）、u2（3211）各开一份种子图（一个容器 P-1001），连同一个服务器（3240）。
在线时 u1 改一次设计压力，两边到修订 1。

OS-03：杀掉服务器；u1 离线改三步（设计压力 1.6MPa、材质 不锈钢、设计压力 2.5MPa——
同一个字段改两次，次序一反结果就不同），保存、导出队列、退出（答「还有 n 步没有同步」的
提示）。不连协同的实例读回存盘文件：同步记录是这台服务器的库标识和修订 1，队列是
这三步，编号和次序与导出的一样。服务器用同一个库起来，u2 连回去改设计温度（修订 2）。
u1 重开：hello 带修订 1 和库标识，修订 2 先写进本图，然后才发自己的；三步按原来的编号、
次序和基线（修订 1）发出，成为修订 3、4、5。两端的图逐项相同。

OS-04：u1 保存（队列已空）；杀掉服务器，u1 退出，在没有服务器的时候重开：状态是
offline，改一步就排进队列。服务器起来后这一步提交为修订 6，两端相同。

证据：logs/o01-*.json、logs/gui-o01-*.err.log、logs/o01-srv.out.log、
shots/offline-o01-u1.png、shots/offline-o01-u2.png。
"""
import sys

import m7h
import oh
import yh

R = m7h.Results("o01_restart_resubmit")
PRESSURE, TEMPERATURE, MATERIAL = "设计压力", "设计温度", "材质"
STEPS = [(PRESSURE, "1.6MPa"), (MATERIAL, "不锈钢"), (PRESSURE, "2.5MPa")]


def change_of(entry):
    first = (entry.get("changes") or [{}])[0]
    return first.get("field"), first.get("value")


try:
    if not m7h.SEED_DWG.exists():
        R.check("setup", False, f"种子图不存在：{m7h.SEED_DWG}")
        raise SystemExit(1)
    dwg_a = m7h.fresh_copy("o01-PID-1101-a.dwg", m7h.SEED_DWG)
    dwg_b = m7h.fresh_copy("o01-PID-1101-b.dwg", m7h.SEED_DWG)
    srv = m7h.Server(3240, tag="o01-srv").start(fresh=True)
    R.check("setup", srv.wait_health(), f"服务器 /health = {srv.health()}")
    u1 = m7h.Gui("o01-u1", 3210, dwg=dwg_a, user="u1").start()
    u2 = m7h.Gui("o01-u2", 3211, dwg=dwg_b, user="u2").start()
    R.check("setup", u1.wait_ready(180) is not None and u2.wait_ready(180) is not None, "两个 GUI 就绪")
    s1 = u1.wait_status(lambda s: s.get("connected") and s.get("session_id"), 30)
    s2 = u2.wait_status(lambda s: s.get("connected") and s.get("session_id"), 30)
    R.check("setup", s1 is not None and s2 is not None, "u1、u2 均已连接")
    for g in (u1, u2):
        g.run("ZOOM EXTENTS")
    resp = oh.set_prop(u1, PRESSURE, "1.0MPa")
    at1 = all(g.wait_status(lambda s: s.get("base_revision") == 1 and s.get("queued_ops") == 0, 15) is not None
              for g in (u1, u2))
    repo = u1.status().get("repo_id")
    R.check("setup", yh.ok(resp) and at1 and oh.value(u2, PRESSURE) == "1.0MPa" and bool(repo),
            f"在线时 u1 改设计压力为 1.0MPa：两边到修订 1，u2 读回 {oh.value(u2, PRESSURE)}；库标识 {repo}")

    # ── OS-03：离线改三步，保存、退出 ──────────────────────────────────────────
    srv.kill()
    R.check("setup", oh.away(u1) and oh.away(u2), "服务器被杀掉后两边都报告未连接、没有会话")
    answers = [oh.set_prop(u1, field, text) for field, text in STEPS]
    q1 = u1.wait_status(lambda s: s.get("queued_ops") == 3, 10) or u1.status()
    R.dump("o01-offline-status.json", q1)
    R.check("setup", all(yh.ok(a) for a in answers) and oh.value(u1, PRESSURE) == "2.5MPa"
            and oh.value(u2, PRESSURE) == "1.0MPa",
            f"离线时 u1 改三步，命令都成功：u1 的设计压力 {oh.value(u1, PRESSURE)}、材质 {oh.value(u1, MATERIAL)}，"
            f"u2 仍是 {oh.value(u2, PRESSURE)}；状态 queued_ops={q1.get('queued_ops')} align={q1.get('align')}")
    saved = oh.save(u1)
    queue = oh.export(u1, m7h.WORK / "o01-queue.json")
    R.dump("o01-queue-export.json", queue)
    ids = oh.queued_ids(queue)
    entries = [e for e in (queue or {}).get("entries") or [] if e.get("state") == "queued"]
    R.check("setup", saved[0] and len(ids) == 3 and len(set(ids)) == 3,
            f"u1 保存（POST /save -> {saved[1]}）并导出队列：{[change_of(e) for e in entries]}，编号 {ids}")
    gone, how = oh.quit_gui(u1)
    R.check("setup", gone, f"u1 退出：{how}")
    on_file = oh.read_file("o01-file", 3212, dwg_a)
    R.dump("o01-file-after-offline.json", on_file)
    claim = oh.claim_of(on_file)
    in_file = oh.file_queue(on_file)
    R.check("OS-03", claim == ("PID-1101", repo, 1, None) and [i for i, _ in in_file] == ids
            and all(user == "u1" for _, user in in_file) and [change_of({"changes": [e.get("payload") or {}]})
                                                             for e in on_file.get(oh.QUEUE) or []] == STEPS,
            f"存盘文件里：同步记录 {claim}（应为 PID-1101、库标识 {repo}、修订 1、没有不一致标记），"
            f"队列 {in_file}（应为导出的三步，同样的编号和次序，排队的人 u1）")

    # ── OS-03：房间往前走一步，u1 重开 ───────────────────────────────────────────
    srv.start(fresh=False)
    R.check("setup", srv.wait_health(), f"服务器用同一个库重新起来：/health = {srv.health()}")
    b2 = oh.joined(u2)
    resp = oh.set_prop(u2, TEMPERATURE, "120℃")
    at2 = u2.wait_status(lambda s: s.get("base_revision") == 2 and s.get("queued_ops") == 0, 15)
    R.check("setup", b2 is not None and yh.ok(resp) and at2 is not None and oh.server_value(srv, TEMPERATURE) == "120℃",
            f"u2 连回去改设计温度为 120℃：房间到修订 {(at2 or {}).get('base_revision')}，服务器读回 "
            f"{oh.server_value(srv, TEMPERATURE)}")
    R.check("setup", oh.reopen(u1), "u1 重开同一份图")
    back = oh.joined(u1, 90, also=lambda s: s.get("base_revision") == 5)
    R.dump("o01-reopened-status.json", back or u1.status())
    frames = oh.wire(u1)
    R.dump("o01-reopened-wire.json", frames)
    hellos = [f for d, f in frames if d == "->" and f.get("type") == "hello"]
    welcomes = [f for d, f in frames if d == "<-" and f.get("type") == "welcome"]
    first_op = next((n for n, (d, f) in enumerate(frames) if d == "->" and f.get("type") == "op"), None)
    caught = next((n for n, (d, f) in enumerate(frames) if d == "<-" and f.get("type") == "op_applied"
                   and f.get("revision") == 2), None)
    R.check("OS-03", bool(hellos) and hellos[-1].get("since") == 1 and hellos[-1].get("repo_id") == repo
            and bool(welcomes) and welcomes[-1].get("replay_from") == 1 and not welcomes[-1].get("align_refused")
            and caught is not None and first_op is not None and caught < first_op
            and oh.value(u1, TEMPERATURE) == "120℃",
            f"重开后先补齐：hello 带 since={hellos[-1].get('since') if hellos else None}、"
            f"repo_id={'对' if hellos and hellos[-1].get('repo_id') == repo else '不对'}，welcome "
            f"replay_from={welcomes[-1].get('replay_from') if welcomes else None}；修订 2 的 op_applied 在第 {caught} 帧，"
            f"第一条自己的 op 在第 {first_op} 帧；u1 的设计温度 {oh.value(u1, TEMPERATURE)}")
    sent = [f for d, f in frames if d == "->" and f.get("type") == "op"]
    rows = oh.ops_rows(srv)
    R.dump("o01-ops-after-resubmit.json", rows)
    mine = [r for r in rows if r["revision"] in (3, 4, 5)]
    R.check("OS-03", [f.get("op_id") for f in sent] == ids and all(f.get("base_revision") == 1 for f in sent)
            and [r["op_id"] for r in mine] == ids and all(r["user_id"] == "u1" and r["base_revision"] == 1 for r in mine)
            and len(rows) == 5,
            f"三步按原来的编号和次序、带原来的基线发出：发出 {[(f.get('op_id'), f.get('base_revision')) for f in sent]}；"
            f"服务器日志 {[(r['revision'], r['op_id'], r['user_id'], r['base_revision']) for r in rows]}")
    a1, a2, same = yh.wait_same(u1, u2, 20)
    R.dump("o01-drawings-after-resubmit.json", {"u1": a1["authored"], "u2": a2["authored"]})
    fine, why = yh.not_diverged(u1, u2)
    values = [(oh.value(g, PRESSURE), oh.value(g, MATERIAL), oh.value(g, TEMPERATURE)) for g in (u1, u2)]
    on_server = (oh.server_value(srv, PRESSURE), oh.server_value(srv, MATERIAL), oh.server_value(srv, TEMPERATURE))
    R.check("OS-03", same and fine and values[0] == values[1] == on_server == ("2.5MPa", "不锈钢", "120℃")
            and not u1.status().get("rejections"),
            f"两端的图逐项相同（位号、属性、拓扑）：{same}；u1 / u2 / 服务器的设计压力、材质、设计温度 "
            f"{values[0]} / {values[1]} / {on_server}；没有被拒的，没有不一致：{why}")

    # ── OS-04：没有服务器时打开 ────────────────────────────────────────────────────
    saved = oh.save(u1)
    srv.kill()
    R.check("setup", saved[0] and oh.away(u1) and oh.away(u2), "u1 保存（队列已空），服务器再杀掉，两边都未连接")
    gone, how = oh.quit_gui(u1)
    R.check("setup", gone and oh.reopen(u1), f"u1 退出（{how}），在没有服务器的时候重开")
    resp = oh.set_prop(u1, PRESSURE, "3.2MPa")
    q1 = u1.wait_status(lambda s: s.get("queued_ops") == 1, 10) or u1.status()
    R.dump("o01-started-offline-status.json", q1)
    R.check("OS-04", yh.ok(resp) and q1.get("connected") is False and q1.get("align") == "offline"
            and q1.get("queued_ops") == 1 and oh.value(u1, PRESSURE) == "3.2MPa",
            f"没有服务器时打开带记录的图：align={q1.get('align')}，改一步后 queued_ops={q1.get('queued_ops')}，"
            f"本图设计压力 {oh.value(u1, PRESSURE)}")
    srv.start(fresh=False)
    R.check("setup", srv.wait_health(), "服务器又用同一个库起来")
    back = oh.joined(u1, 90, also=lambda s: s.get("base_revision") == 6)
    oh.joined(u2, 90, also=lambda s: s.get("base_revision") == 6)
    rows = oh.ops_rows(srv)
    R.dump("o01-ops-after-offline-start.json", rows)
    sent = oh.sent_ops(u1)
    a1, a2, same = yh.wait_same(u1, u2, 20)
    fine, why = yh.not_diverged(u1, u2)
    R.check("OS-04", back is not None and len(rows) == 6 and rows[-1]["user_id"] == "u1"
            and [f.get("op_id") for f in sent] == [rows[-1]["op_id"]] and same and fine
            and oh.value(u2, PRESSURE) == oh.server_value(srv, PRESSURE) == "3.2MPa",
            f"服务器起来后这一步提交：服务器日志最后一行 {rows[-1] if rows else None}，u1 发出 "
            f"{[f.get('op_id') for f in sent]}；u2 读回 {oh.value(u2, PRESSURE)}，服务器 {oh.server_value(srv, PRESSURE)}；"
            f"两端相同 {same}，没有不一致：{why}")
    for name, g in (("u1", u1), ("u2", u2)):
        st, _resp, shot = g.capture(m7h.SHOTS / f"offline-o01-{name}.png")
        R.note(f"{name} 截图 shots/offline-o01-{name}.png：HTTP {st}，文件{'在' if shot else '不在'}")
except SystemExit:
    raise
except Exception as error:  # noqa: BLE001
    import traceback
    R.check("script", False, f"脚本异常: {error!r}\n{traceback.format_exc()}")
finally:
    m7h.cleanup()
    ok = R.summary()
sys.exit(0 if ok else 1)
