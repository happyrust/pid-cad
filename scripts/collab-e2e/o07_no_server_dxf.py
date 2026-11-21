# -*- coding: utf-8 -*-
"""OS-01、OS-02（SPEC-collab-offline-sync.md §7、§8）：没有服务器时打开、绘制、保存、重开；三条记录经 DWG 和 DXF 存盘读回。

准备（造一份三条记录都有的图）：服务器（3240，新库），u1（3210）打开种子图 A 加入，在线改设计压力为
1.0MPa（修订 1）。杀掉服务器，u1 离线改设计温度为 150℃（排队一步），保存、退出。服务器用同一个库起来，
一个 WS 会话（u2）占住 P-1001；u1 重开 A，补交的那一步被拒（CLAIM_HELD）并撤回，记进未同步清单。
关掉 WS 会话、杀掉服务器，u1 离线改备注（排队一步），保存、退出。这时 A 里有三条记录：同步记录
（PID-1101、库标识、修订 1）、队列一条、未同步清单一条（被拒、已撤回）。

OS-01：服务器不在。u1 带着协同参数打开 A（连不上）：状态 align=offline，队列 1 条、清单 1 条；画一条直线、
一个圆，放一个阀门，改一个属性，保存，每一步都成功（失败步骤数 0）；退出、再打开（仍没有服务器）：画的
东西都在，队列多了放阀门和改属性两步。不连协同的实例读文件：同步记录与打开前逐项相同，队列是原来那条
（编号不变）加新排的两条，清单与打开前相同。再用不连协同的实例打开 A，画一条直线、一个圆，保存，每一步
都成功；读文件：三条记录与这之前逐项相同。

OS-02：u1（协同参数，没有服务器）打开 A，经控制接口另存成 DWG 和 DXF 各一份；不连协同的实例分别读这两份：
三条记录与 A 里的逐项相同。u1 再带着协同参数打开那份 DXF：状态读出的队列条数、清单条数与 A 的相同，
仍按离线（align=offline）对待。

证据：logs/o07-*.json、logs/gui-o07-*.err.log、logs/o07-srv.out.log、logs/ws-o07-*.jsonl，
shots/offline-o07-u1.png。
"""
import sys

import m7h
import oh
import yh

R = m7h.Results("o07_no_server_dxf")
PRESSURE, TEMPERATURE, NOTE = "设计压力", "设计温度", "备注"


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


def count(gui, kind):
    return len((gui.op("query", type=kind)[1] or {}).get("entities") or [])


def three(found):
    """The three records of a file as read, {key: items}; a key the file lacks reads None."""
    return {key: (found or {}).get(key) for key in oh.RECORDS}


def offline(gui, seconds=20):
    return gui.wait_status(lambda s: s.get("connected") is False and s.get("align") == "offline", seconds)


def save_as(gui, path):
    if path.exists():
        path.unlink()
    st, body = gui.api("POST", "/save", {"document_id": gui.doc_id(), "path": str(path)})
    m7h.wait_until(path.exists, 10)
    return st == 200 and (body or {}).get("ok") is True and path.exists(), st


try:
    if not m7h.SEED_DWG.exists():
        R.check("setup", False, f"种子图不存在：{m7h.SEED_DWG}")
        raise SystemExit(1)
    dwg_a = m7h.fresh_copy("o07-PID-1101-a.dwg", m7h.SEED_DWG)
    srv = m7h.Server(3240, tag="o07-srv").start(fresh=True)
    R.check("setup", srv.wait_health(), f"服务器 /health = {srv.health()}")
    u1 = started("o07-u1", 3210, dwg_a, user="u1")
    resp = None
    if u1.wait_status(lambda s: s.get("connected") and s.get("session_id"), 30):
        resp = oh.set_prop(u1, PRESSURE, "1.0MPa")
    at1 = u1.wait_status(lambda s: s.get("base_revision") == 1 and s.get("queued_ops") == 0, 15)
    repo = u1.status().get("repo_id")
    original = oh.value(u1, TEMPERATURE)
    R.check("setup", resp is not None and yh.ok(resp) and at1 is not None and bool(repo),
            f"u1 加入并在线改设计压力为 1.0MPa：到修订 1，库标识 {repo}；设计温度原来是 {original!r}")
    srv.kill()
    R.check("setup", oh.away(u1), "杀掉服务器；u1 未连接")
    resp = oh.set_prop(u1, TEMPERATURE, "150℃")
    R.check("setup", yh.ok(resp) and u1.status().get("queued_ops") == 1, "u1 离线改设计温度为 150℃，排队一步")
    finish(u1, "A：离线改了设计温度")
    srv.start(fresh=False)
    R.check("setup", srv.wait_health(), "服务器用同一个库重新起来")
    ws, welcome = oh.ws_join(srv, "o07-u2", "u2")
    granted = claim(ws, oh.OBJ) if ws.session_id else None
    R.check("setup", (granted or {}).get("type") == "claim_granted",
            f"WS 会话（u2）加入并占住 {oh.OBJ}：{(granted or {}).get('type')}")
    R.check("setup", oh.reopen(u1), "u1 重开 A")
    refused = u1.wait_status(lambda s: s.get("connected") and s.get("queued_ops") == 0
                             and s.get("unsynced_count") == 1, 60)
    status = refused or u1.status()
    codes = [r.get("code") for r in status.get("rejections") or []]
    R.check("setup", refused is not None and "CLAIM_HELD" in codes and oh.value(u1, TEMPERATURE) == original,
            f"补交的那一步被拒（拒绝记录的码 {codes}）并撤回（设计温度 {oh.value(u1, TEMPERATURE)!r}），"
            f"unsynced_count={status.get('unsynced_count')}")
    ws.close()
    srv.kill()
    R.check("setup", oh.away(u1), "关掉 WS 会话、杀掉服务器；u1 未连接")
    resp = oh.set_prop(u1, NOTE, "离线一")
    R.check("setup", yh.ok(resp) and u1.status().get("queued_ops") == 1, "u1 离线改备注为「离线一」，排队一步")
    finish(u1, "A：离线改了备注")
    before = oh.read_file("o07-a-file0", 3211, dwg_a)
    queue0 = oh.file_queue(before)
    unsynced0 = before.get(oh.UNSYNCED) or []
    R.dump("o07-a-before.json", before)
    R.check("setup", oh.claim_of(before) == (oh.DRAWING, repo, 1, None) and len(queue0) == 1
            and [(e.get("state"), e.get("code"), e.get("rolled_back")) for e in unsynced0] == [("refused", "CLAIM_HELD", True)],
            f"A 里的三条记录：同步记录 {oh.claim_of(before)}，队列 {queue0}，未同步清单 "
            f"{[(e.get('state'), e.get('code'), e.get('rolled_back')) for e in unsynced0]}")

    # ── OS-01：没有服务器时打开、绘制、保存、重开 ──────────────────────────────────────
    R.check("setup", not m7h.port_listening(srv.port), f"服务器不在：{srv.port} 上没有人在听")
    R.check("setup", oh.reopen(u1), "u1 带着协同参数重开 A（没有服务器）")
    first = offline(u1)
    status = first or u1.status()
    lines0, circles0, known = count(u1, "Line"), count(u1, "Circle"), set(u1.tagged())
    steps = [("LINE", yh.draw(u1, "LINE", [(-400.0, -400.0), (-300.0, -300.0)])),
             ("CIRCLE", u1.run("CIRCLE -350,-200 25")),
             ("PIDINSERT", u1.run("PIDINSERT VALVE 400,200")),
             ("PIDSETPROP", oh.set_prop(u1, NOTE, "离线二"))]
    saved = oh.save(u1)
    failed = [name for name, resp in steps if not yh.ok(resp)] + ([] if saved[0] else [f"保存（{saved[1]}）"])
    valves = sorted(set(u1.tagged()) - known)
    drawn = u1.status()
    R.dump("o07-os01-offline.json", {"opened": status, "after": drawn, "steps": {n: yh.body_of(r) for n, r in steps}})
    R.check("OS-01", first is not None and status.get("queued_ops") == 1 and status.get("unsynced_count") == 1
            and not failed and len(valves) == 1 and drawn.get("queued_ops") == 3,
            f"没有服务器，u1 带着协同参数打开 A：align={status.get('align')}，队列 {status.get('queued_ops')} 条、"
            f"清单 {status.get('unsynced_count')} 条；画直线、画圆、放阀门（{valves}）、改备注、保存，失败的步骤 {failed}"
            f"（{len(failed)} 步）；之后队列 {drawn.get('queued_ops')} 条")
    shot = m7h.SHOTS / "offline-o07-u1.png"
    st, _resp, _shot = u1.capture(shot)
    R.note(f"u1 截图 shots/{shot.name}：HTTP {st}，文件{'在' if shot.exists() else '不在'}")
    gone, how = oh.quit_gui(u1)
    R.check("setup", gone, f"u1 退出（{how}）")
    R.check("setup", oh.reopen(u1), "u1 再带着协同参数打开 A（仍没有服务器）")
    again = offline(u1) or u1.status()
    kept = (count(u1, "Line"), count(u1, "Circle"), bool(valves) and valves[0] in u1.tagged(), oh.value(u1, NOTE))
    R.check("OS-01", kept == (lines0 + 1, circles0 + 1, True, "离线二") and again.get("queued_ops") == 3
            and again.get("unsynced_count") == 1 and again.get("align") == "offline",
            f"重开后画的东西都在：直线 {lines0} → {kept[0]}，圆 {circles0} → {kept[1]}，阀门{'在' if kept[2] else '不在'}，"
            f"备注 {kept[3]!r}；align={again.get('align')}，队列 {again.get('queued_ops')} 条、清单 {again.get('unsynced_count')} 条")
    gone, how = oh.quit_gui(u1)
    R.check("setup", gone, f"u1 退出（{how}）")
    on_file = oh.read_file("o07-a-file1", 3211, dwg_a)
    queue1 = oh.file_queue(on_file)
    R.dump("o07-a-after-offline.json", on_file)
    R.check("OS-01", oh.claim_of(on_file) == oh.claim_of(before) and queue1[:1] == queue0 and len(queue1) == 3
            and all(user == "u1" for _op, user in queue1) and on_file.get(oh.UNSYNCED) == unsynced0,
            f"不连协同的实例读文件：同步记录 {oh.claim_of(on_file)}（打开前 {oh.claim_of(before)}），"
            f"队列 {queue1}（原来的 {queue0} 在最前），未同步清单{'与打开前相同' if on_file.get(oh.UNSYNCED) == unsynced0 else '变了'}")

    plain = started("o07-a-plain", 3212, dwg_a, collab=False)
    lines1, circles1 = count(plain, "Line"), count(plain, "Circle")
    steps = [("LINE", yh.draw(plain, "LINE", [(-400.0, 400.0), (-300.0, 300.0)])),
             ("CIRCLE", plain.run("CIRCLE -350,200 25"))]
    saved = oh.save(plain)
    failed = [name for name, resp in steps if not yh.ok(resp)] + ([] if saved[0] else [f"保存（{saved[1]}）"])
    drew = (count(plain, "Line") - lines1, count(plain, "Circle") - circles1)
    gone, how = oh.quit_gui(plain)
    R.check("setup", gone, f"不连协同的实例退出（{how}）")
    plain_file = oh.read_file("o07-a-file2", 3213, dwg_a)
    R.dump("o07-a-after-plain.json", plain_file)
    R.check("OS-01", not failed and drew == (1, 1) and three(plain_file) == three(on_file),
            f"不连协同的实例打开 A，画直线、画圆（多了 {drew[0]} 条线、{drew[1]} 个圆）、保存，失败的步骤 {failed}"
            f"（{len(failed)} 步）；读文件，三条记录与这之前{'逐项相同' if three(plain_file) == three(on_file) else '不同'}")

    # ── OS-02：经 DWG 和 DXF 存盘读回 ──────────────────────────────────────────────────
    R.check("setup", oh.reopen(u1), "u1 带着协同参数打开 A（没有服务器）")
    R.check("setup", offline(u1) is not None, "u1 按离线对待")
    out_dwg, out_dxf = m7h.WORK / "o07-out.dwg", m7h.WORK / "o07-out.dxf"
    as_dwg = save_as(u1, out_dwg)
    as_dxf = save_as(u1, out_dxf)
    gone, how = oh.quit_gui(u1)
    R.check("setup", as_dwg[0] and as_dxf[0] and gone,
            f"u1 另存 {out_dwg.name}（{as_dwg[1]}）、{out_dxf.name}（{as_dxf[1]}），退出（{how}）")
    from_dwg = oh.read_file("o07-dwg-file", 3214, out_dwg)
    from_dxf = oh.read_file("o07-dxf-file", 3215, out_dxf)
    R.dump("o07-os02.json", {"a": three(plain_file), "dwg": three(from_dwg), "dxf": three(from_dxf)})
    differs = [(name, key) for name, found in (("DWG", from_dwg), ("DXF", from_dxf))
               for key in oh.RECORDS if three(found)[key] != three(plain_file)[key]]
    R.check("OS-02", not differs and all(three(plain_file)[key] for key in oh.RECORDS),
            f"不连协同的实例读另存的 DWG 和 DXF：三条记录与 A 里的比，不同的 {differs or '没有'}"
            f"（A 里同步记录 {oh.claim_of(plain_file)}，队列 {len(oh.file_queue(plain_file))} 条，"
            f"清单 {len(plain_file.get(oh.UNSYNCED) or [])} 条）")
    dxf = started("o07-dxf", 3216, out_dxf, user="u1")
    seen = offline(dxf) or dxf.status()
    R.dump("o07-dxf-status.json", seen)
    R.check("OS-02", seen.get("align") == "offline" and seen.get("queued_ops") == len(oh.file_queue(plain_file))
            and seen.get("unsynced_count") == len(plain_file.get(oh.UNSYNCED) or []) and not seen.get("diverged"),
            f"u1 带着协同参数打开那份 DXF（没有服务器）：align={seen.get('align')}，队列 {seen.get('queued_ops')} 条、"
            f"清单 {seen.get('unsynced_count')} 条，diverged={seen.get('diverged')}")
    gone, how = oh.quit_gui(dxf)
    R.check("setup", gone, f"DXF 那个实例退出（{how}）")
except SystemExit:
    raise
except Exception as error:  # noqa: BLE001
    import traceback
    R.check("script", False, f"脚本异常: {error!r}\n{traceback.format_exc()}")
finally:
    m7h.cleanup()
    ok = R.summary()
sys.exit(0 if ok else 1)
