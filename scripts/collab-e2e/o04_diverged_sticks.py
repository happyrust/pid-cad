# -*- coding: utf-8 -*-
"""OS-10 – OS-13（SPEC-collab-offline-sync.md §5、§8）：不一致的图一直按不一致处理；图号和同步记录；别人排的队列。

准备：服务器（3240，新库），u1（3210）打开种子图 A 加入，在线改设计压力为 1.0MPa（修订 1），
保存（图里记着 PID-1101、库标识、修订 1，没有队列），复制成 B、C、D、E。杀掉服务器，u1 离线
改设计温度为 150℃（排队一步），保存 A，复制成 F、G，退出；服务器用同一个库起来。

OS-11：不连协同的实例在 B 上改材质为不锈钢、在 C 上只把 P-1001 挪一个位置，各自保存。B 加入
时状态是不一致（UNQUEUED_EDITS）并说明原因，hello 不报库标识、修订和位号；C 加入时对得上
（hello 带 since=1 和库标识，状态 ok）。

OS-12：D 用 --drawing PID-1102 打开：命令行说「这份图记着它属于 PID-1101」，hello 不拿这份图的
记录说话，保存后读回的同步记录没变。E 不给图号：命令行说图号按同步记录取 PID-1101，
hello 用 PID-1101、since=1，状态 ok。B 在不连协同的实例里 PIDCOLLAB DETACH（有未同步清单，
不加 FORCE 被拒），保存后三条记录都没有了；再加入时 hello 不带库标识和修订，状态 ok。

OS-13：F 由 u2 打开：按对不上处理（QUEUED_BY_OTHERS），一步都不发；保存后队列还是 u1 的那一步。

OS-10：房间里一个 WS 会话改 V-1001（本图没有这个位号）到修订 2；C（没有队列）重开，补齐时
这一步写不进图，标为不一致（REMOTE_UNAPPLIED）；保存后文件带着这个标记，再重开加入仍是不一致，
hello 不报库标识、修订和位号。G（带着 u1 的队列）由 u1 打开，补齐时修订 2 写不进图：标为不一致
（REMOTE_UNAPPLIED），一步都不发，服务器日志不变；保存后文件里标记和队列都在（补发的修改写进图
之前不发队列，见 SPEC-collab-offline-sync.md §11.16）。A 带着 u1 的队列，不连协同的实例又在上面
改了材质并保存（标 UNQUEUED_EDITS）；u1 在会话里打开 A：不一致，一步都不发；在会话里保存后文件
里标记和队列都在；再重开加入：仍是不一致，hello 什么都不报，一步都不发，服务器日志不变。

证据：logs/o04-*.json、logs/gui-o04-*.err.log、logs/o04-srv.out.log、logs/ws-o04-*.jsonl，
shots/offline-o04-u1.png。
"""
import sys
import time

import m7h
import oh
import yh

R = m7h.Results("o04_diverged_sticks")
PRESSURE, TEMPERATURE, MATERIAL = "设计压力", "设计温度", "材质"


def started(name, port, dwg, **more):
    gui = m7h.Gui(name, port, dwg=dwg, **more).start()
    plain = "（不连协同）" if more.get("collab") is False else ""
    R.check("setup", gui.wait_ready(180) is not None, f"{name}{plain} 打开 {dwg.name}")
    return gui


def finish(gui, what):
    saved = oh.save(gui)
    gone, how = oh.quit_gui(gui)
    R.check("setup", saved[0] and gone, f"{what}：保存（{saved[1]}）、退出（{how}）")


def entity(gui, handle):
    return next((e for e in gui.inserts() if e.get("handle") == handle), None)


def in_session(gui, align, seconds=60):
    """Connected, in a session, and the status says `align`; that status, or None."""
    return oh.held_apart(gui, align, seconds)


def keys_of(hellos):
    return [sorted(k for k in h if k in ("drawing_id", "repo_id", "since", "existing_tags")) for h in hellos]


try:
    if not m7h.SEED_DWG.exists():
        R.check("setup", False, f"种子图不存在：{m7h.SEED_DWG}")
        raise SystemExit(1)
    dwg_a = m7h.fresh_copy("o04-PID-1101-a.dwg", m7h.SEED_DWG)
    srv = m7h.Server(3240, tag="o04-srv").start(fresh=True)
    R.check("setup", srv.wait_health(), f"服务器 /health = {srv.health()}")
    u1 = started("o04-u1", 3210, dwg_a, user="u1")
    resp = None
    if u1.wait_status(lambda s: s.get("connected") and s.get("session_id"), 30):
        resp = oh.set_prop(u1, PRESSURE, "1.0MPa")
    at1 = u1.wait_status(lambda s: s.get("base_revision") == 1 and s.get("queued_ops") == 0, 15)
    repo = u1.status().get("repo_id")
    saved = oh.save(u1)
    R.check("setup", resp is not None and yh.ok(resp) and at1 is not None and bool(repo) and saved[0],
            f"u1 加入并在线改设计压力为 1.0MPa：到修订 1，库标识 {repo}；保存（{saved[1]}）")
    copies = {k: m7h.fresh_copy(f"o04-{k}.dwg", dwg_a) for k in "bcde"}
    srv.kill()
    R.check("setup", oh.away(u1), "复制出 B、C、D、E（都没有队列），杀掉服务器；u1 未连接")
    resp = oh.set_prop(u1, TEMPERATURE, "150℃")
    saved = oh.save(u1)
    exported = oh.export(u1, m7h.WORK / "o04-queue.json")
    queued = oh.queued_ids(exported)
    gone, how = oh.quit_gui(u1)
    dwg_f = m7h.fresh_copy("o04-f.dwg", dwg_a)
    dwg_g = m7h.fresh_copy("o04-g.dwg", dwg_a)
    R.check("setup", yh.ok(resp) and saved[0] and len(queued) == 1 and gone,
            f"u1 离线改设计温度为 150℃，排队 {queued}，保存 A、退出（{how}），A 复制为 F、G")
    srv.start(fresh=False)
    R.check("setup", srv.wait_health(), "服务器用同一个库重新起来")

    # ── OS-11：不连协同时的语义修改和只挪位置 ───────────────────────────────────────────
    plain_b = started("o04-b-plain", 3211, copies["b"], collab=False)
    resp = oh.set_prop(plain_b, MATERIAL, "不锈钢")
    kept = oh.told_with(plain_b, "未同步清单")
    finish(plain_b, "B：不连协同改材质为不锈钢")
    plain_c = started("o04-c-plain", 3212, copies["c"], collab=False)
    handle = plain_c.tagged().get(oh.OBJ)
    before = entity(plain_c, handle)
    st, body = plain_c.op("entities_transform", handles=[handle], action="move", vector=[25.0, 10.0, 0.0])
    after = entity(plain_c, handle)
    kept_c = oh.told_with(plain_c, "未同步清单")
    finish(plain_c, "C：不连协同只挪 P-1001 的位置")
    R.check("setup", yh.ok(resp) and bool(kept) and st == 200 and before is not None and before != after and not kept_c,
            f"B 改属性后命令行说记进了未同步清单（{len(kept)} 行）；C 挪位置 entities_transform -> {st}，"
            f"位置{'变了' if before != after else '没变'}，命令行说未同步清单 {len(kept_c)} 行")

    b = started("o04-b", 3213, copies["b"], user="u1")
    held = in_session(b, "diverged")
    status = held or b.status()
    said = oh.hellos(b)
    R.dump("o04-b-status.json", {"status": status, "hellos": said, "told": oh.told(b)})
    R.check("OS-11", held is not None and status.get("align_code") == "UNQUEUED_EDITS" and bool(status.get("align_message"))
            and bool(said) and all(oh.claims_nothing(h) for h in said) and not oh.sent_ops(b),
            f"B（不连协同时改过属性）加入：align={status.get('align')} align_code={status.get('align_code')}，"
            f"原因「{status.get('align_message')}」；hello 里的键 {keys_of(said)}；发出的 op {len(oh.sent_ops(b))} 条")
    gone, how = oh.quit_gui(b)
    R.check("setup", gone, f"B 退出（{how}）")
    c = started("o04-c", 3214, copies["c"], user="u1")
    ok_c = oh.joined(c, 60, also=lambda s: s.get("align") == "ok")
    status = ok_c or c.status()
    hello = (oh.hellos(c) or [{}])[-1]
    R.check("OS-11", ok_c is not None and hello.get("since") == 1 and hello.get("repo_id") == repo and not status.get("diverged"),
            f"C（不连协同时只挪了位置）加入：align={status.get('align')} align_code={status.get('align_code')}，"
            f"hello since={hello.get('since')} repo_id {'与图里的相同' if hello.get('repo_id') == repo else hello.get('repo_id')}")
    gone, how = oh.quit_gui(c)
    R.check("setup", gone, f"C 退出（{how}）")

    # ── OS-12：图号、同步记录和 PIDCOLLAB DETACH ───────────────────────────────────────────────────
    d = started("o04-d", 3215, copies["d"], user="u1", drawing="PID-1102")
    m7h.wait_until(lambda: bool(oh.told_with(d, "这份图记着它属于 PID-1101")), 30)
    time.sleep(3)
    warned = oh.told_with(d, "这份图记着它属于 PID-1101")
    status = d.status()
    said = oh.hellos(d)
    finish(d, "D")
    on_file = oh.read_file("o04-d-file", 3216, copies["d"])
    R.dump("o04-d.json", {"status": status, "hellos": said, "file": on_file})
    R.check("OS-12", bool(warned) and not any("repo_id" in h or "since" in h for h in said)
            and oh.claim_of(on_file) == ("PID-1101", repo, 1, None) and not oh.file_queue(on_file),
            f"D（记着 PID-1101，用 --drawing PID-1102 打开）：命令行「{warned[-1] if warned else '（没有）'}」，"
            f"最近错误「{status.get('last_error')}」；hello 里的键 {keys_of(said)}；保存后文件里的同步记录 "
            f"{oh.claim_of(on_file)}（应仍是 PID-1101、原来的库标识、修订 1），队列 {oh.file_queue(on_file)}")

    e = started("o04-e", 3217, copies["e"], user="u1", drawing=None)
    ok_e = oh.joined(e, 60, also=lambda s: s.get("align") == "ok")
    status = ok_e or e.status()
    hello = (oh.hellos(e) or [{}])[-1]
    took = oh.told_with(e, "按这份图的同步记录取 PID-1101")
    R.check("OS-12", ok_e is not None and hello.get("drawing_id") == "PID-1101" and hello.get("since") == 1
            and hello.get("repo_id") == repo and bool(took),
            f"E（不给图号）：命令行「{took[-1] if took else '（没有）'}」；hello drawing_id={hello.get('drawing_id')} "
            f"since={hello.get('since')} repo_id {'与图里的相同' if hello.get('repo_id') == repo else hello.get('repo_id')}；"
            f"align={status.get('align')}")
    gone, how = oh.quit_gui(e)
    R.check("setup", gone, f"E 退出（{how}）")

    detach = started("o04-b-detach", 3218, copies["b"], collab=False)
    detach.run("PIDCOLLAB DETACH")
    refused = oh.told_with(detach, "PIDCOLLAB DETACH", "FORCE", "找不回")
    detach.run("PIDCOLLAB DETACH FORCE")
    done = oh.told_with(detach, "去掉了这张图的同步记录")
    R.check("setup", bool(refused) and bool(done),
            f"B 有未同步清单：PIDCOLLAB DETACH 被拒（{refused[-1] if refused else '没有被拒'}），"
            f"DETACH FORCE：{done[-1] if done else '没有去掉'}")
    finish(detach, "B：DETACH FORCE 之后")
    on_file = oh.read_file("o04-b-file", 3219, copies["b"])
    left = [k for k in oh.RECORDS if k in on_file]
    again = started("o04-b-again", 3223, copies["b"], user="u1")
    ok_b = oh.joined(again, 60, also=lambda s: s.get("align") == "ok")
    status = ok_b or again.status()
    hello = (oh.hellos(again) or [{}])[-1]
    R.dump("o04-b-detached.json", {"file": on_file, "status": status, "hello": hello})
    R.check("OS-12", not left and ok_b is not None and bool(hello) and "repo_id" not in hello and "since" not in hello,
            f"B DETACH FORCE 并保存后文件里剩下的记录 {left}；再加入：hello 里的键 {keys_of([hello])}，"
            f"align={status.get('align')}")
    gone, how = oh.quit_gui(again)
    R.check("setup", gone, f"B 退出（{how}）")

    # ── OS-13：别人排的队列 ──────────────────────────────────────────────────────────────────
    rows = oh.ops_rows(srv)
    f = started("o04-f", 3220, dwg_f, user="u2")
    held = in_session(f, "refused")
    time.sleep(2)
    status = held or f.status()
    R.check("OS-13", held is not None and status.get("align_code") == "QUEUED_BY_OTHERS" and not oh.sent_ops(f)
            and oh.ops_rows(srv) == rows,
            f"F（u1 排的队列）由 u2 打开：align={status.get('align')} align_code={status.get('align_code')}，"
            f"原因「{status.get('align_message')}」；发出的 op {len(oh.sent_ops(f))} 条，服务器日志 {len(rows)} → "
            f"{len(oh.ops_rows(srv))} 行")
    finish(f, "F")
    on_file = oh.read_file("o04-f-file", 3221, dwg_f)
    R.check("OS-13", oh.file_queue(on_file) == [(queued[0], "u1")],
            f"F 保存后文件里的队列 {oh.file_queue(on_file)}（应仍是 u1 排的 {queued}）")

    # ── OS-10：不一致的标记跨保存、重开 ──────────────────────────────────────────────────────
    room, welcome = oh.ws_join(srv, "o04-room", "u9")
    other = oh.ws_write(room, (welcome or {}).get("base_revision") or 1, "op-o04-room-v", PRESSURE, "0.6MPa", obj="V-1001")
    room.close()
    rows = oh.ops_rows(srv)
    R.check("setup", (other or {}).get("type") == "op_applied" and (other or {}).get("revision") == 2,
            f"房间里改 V-1001（本图没有这个位号）：{(other or {}).get('type')} 修订 {(other or {}).get('revision')}")
    cr = started("o04-c-replay", 3224, copies["c"], user="u1")
    apart = cr.wait_status(lambda s: s.get("align") == "diverged", 60)
    status = apart or cr.status()
    R.dump("o04-c-replay.json", {"status": status, "hellos": oh.hellos(cr)})
    R.check("setup", apart is not None and status.get("align_code") == "REMOTE_UNAPPLIED",
            f"C（没有队列）重开补齐时修订 2 写不进图：align={status.get('align')} align_code={status.get('align_code')}，"
            f"原因「{status.get('align_message')}」")
    finish(cr, "C：标了不一致之后")
    on_file = oh.read_file("o04-c-file", 3225, copies["c"])
    claim = oh.claim_of(on_file) or (None, None, None, None)
    again = started("o04-c-again", 3226, copies["c"], user="u1")
    still = in_session(again, "diverged")
    time.sleep(2)
    status = still or again.status()
    said = oh.hellos(again)
    R.dump("o04-c-again.json", {"file": on_file, "status": status, "hellos": said})
    R.check("OS-10", (claim[3] or {}).get("code") == "REMOTE_UNAPPLIED" and still is not None
            and status.get("align_code") == "REMOTE_UNAPPLIED" and bool(said) and all(oh.claims_nothing(h) for h in said),
            f"C 保存后文件里的不一致标记 {(claim[3] or {}).get('code')}；再重开加入：align={status.get('align')} "
            f"align_code={status.get('align_code')}，hello 里的键 {keys_of(said)}")
    gone, how = oh.quit_gui(again)
    R.check("setup", gone, f"C 退出（{how}）")

    rows = oh.ops_rows(srv)
    g = started("o04-g", 3228, dwg_g, user="u1")
    apart = in_session(g, "diverged")
    time.sleep(2)
    status = apart or g.status()
    sent = oh.sent_ops(g)
    said = oh.hellos(g)
    finish(g, "G：标了不一致之后")
    on_file = oh.read_file("o04-g-file", 3229, dwg_g)
    claim = oh.claim_of(on_file) or (None, None, None, None)
    R.dump("o04-g.json", {"status": status, "hellos": said, "file": on_file})
    R.check("OS-10", apart is not None and status.get("align_code") == "REMOTE_UNAPPLIED" and not sent
            and oh.ops_rows(srv) == rows and (claim[3] or {}).get("code") == "REMOTE_UNAPPLIED"
            and oh.file_queue(on_file) == [(queued[0], "u1")],
            f"G（带着 u1 的队列）由 u1 打开，补齐时修订 2 写不进图：align={status.get('align')} "
            f"align_code={status.get('align_code')}，发出的 op {len(sent)} 条，服务器日志 {len(rows)} → "
            f"{len(oh.ops_rows(srv))} 行；保存后文件里的不一致标记 {(claim[3] or {}).get('code')}，"
            f"队列 {oh.file_queue(on_file)}（应仍是 {queued}）")

    plain_a = started("o04-a-plain", 3227, dwg_a, collab=False)
    resp = oh.set_prop(plain_a, MATERIAL, "碳钢")
    finish(plain_a, "A（带着 u1 的队列）：不连协同改材质为碳钢")
    rows = oh.ops_rows(srv)
    R.check("setup", yh.ok(resp) and oh.reopen(u1), "u1 重开 A")
    first = in_session(u1, "diverged")
    time.sleep(2)
    status = first or u1.status()
    sent = oh.sent_ops(u1)
    finish(u1, "A：在会话里保存")
    on_file = oh.read_file("o04-a-file", 3222, dwg_a)
    claim = oh.claim_of(on_file) or (None, None, None, None)
    R.dump("o04-a-file.json", {"status": status, "file": on_file})
    R.check("OS-10", first is not None and status.get("align_code") == "UNQUEUED_EDITS" and not sent
            and (claim[3] or {}).get("code") == "UNQUEUED_EDITS" and oh.file_queue(on_file) == [(queued[0], "u1")],
            f"A（u1 的队列 + 不连协同时的修改）在会话里打开：align={status.get('align')} "
            f"align_code={status.get('align_code')}，发出的 op {len(sent)} 条；在会话里保存后文件里的不一致标记 "
            f"{(claim[3] or {}).get('code')}，队列 {oh.file_queue(on_file)}（应仍是 {queued}）")
    R.check("setup", oh.reopen(u1), "u1 再重开 A")
    still = in_session(u1, "diverged")
    time.sleep(2)
    status = still or u1.status()
    said = oh.hellos(u1)
    R.dump("o04-a-again.json", {"status": status, "hellos": said})
    R.check("OS-10", still is not None and status.get("align_code") == "UNQUEUED_EDITS" and bool(said)
            and all(oh.claims_nothing(h) for h in said) and not oh.sent_ops(u1) and oh.ops_rows(srv) == rows,
            f"A 再重开加入：align={status.get('align')} align_code={status.get('align_code')}；hello 里的键 {keys_of(said)}；"
            f"发出的 op {len(oh.sent_ops(u1))} 条；服务器日志 {len(rows)} → {len(oh.ops_rows(srv))} 行；"
            f"queued_ops={status.get('queued_ops')} unsynced_count={status.get('unsynced_count')}")
    shot = m7h.SHOTS / "offline-o04-u1.png"
    st, _resp, _shot = u1.capture(shot)
    R.note(f"u1 截图 shots/{shot.name}：HTTP {st}，文件{'在' if shot.exists() else '不在'}")
except SystemExit:
    raise
except Exception as error:  # noqa: BLE001
    import traceback
    R.check("script", False, f"脚本异常: {error!r}\n{traceback.format_exc()}")
finally:
    m7h.cleanup()
    ok = R.summary()
sys.exit(0 if ok else 1)
