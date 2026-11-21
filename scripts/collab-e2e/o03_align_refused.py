# -*- coding: utf-8 -*-
"""OS-08、OS-09（SPEC-collab-offline-sync.md §7、§8）：三种对不上，和没对上的会话硬发 op。

准备：服务器（3240，新库）第一次起来就结束，把库（修订 0）另存一份，再用同一个库
起来。u1（3210）打开种子图加入，在线改设计压力为 1.0MPa（修订 1）；杀掉服务器，把库
（修订 1）再另存一份；u1 离线改设计温度为 150℃（排队一步），保存、退出。这份图记着
库标识和修订 1，队列里一步；复制成三份，各对一种对不上：

- REPO_MISMATCH：服务器换一个新库（另一个库标识），房间里先有一条别人的修改；
- REVISION_AHEAD：服务器换回修订 0 那份库（同一个库标识，房间落后于图）；
- REPLAY_GAP：服务器换回修订 1 那份库，房间又往前走两步（修订 2、3），停下服务器删掉
  操作日志里修订 2 那一行，再起来。

每一种：一个实例打开对应的副本加入，状态说对不上和原因，命令行说「需要取得最新图纸」；
本地再改一步（材质 铜），照常排队；房间里一个 WS 会话接着改设计压力。一步都没有发
出去，服务器日志只多了 WS 会话的那一步；房间的修改（加入之前的和之后的）都没有写进这份
图。保存、退出，不连协同的实例读回存盘文件：库标识和修订还是原来的，队列是这两步。

OS-09：REPO_MISMATCH 那一轮里，一个 WS 会话 hello 时报一个别的库标识，被判对不上之后
硬发一个 op：服务器回 error，码是 NOT_ALIGNED，带这个 op 的编号、不带 rejected；操作日志
里没有这一步，房间里的值没变，在场的另一个会话没有收到广播。

证据：logs/o03-*.json、logs/gui-o03-*.err.log、logs/o03-*.out.log、logs/ws-o03-*.jsonl。
"""
import sys
import time

import m7h
import oh
import yh

R = m7h.Results("o03_align_refused")
PRESSURE, TEMPERATURE, MATERIAL = "设计压力", "设计温度", "材质"
OWN = ("1.0MPa", "150℃", "铜")


def current(srv):
    return max([r["revision"] for r in oh.ops_rows(srv)], default=0)


def refused(code, srv, dwg, port, room, repo, queued):
    """One copy against a room that does not take it as aligned for `code`.
    `room` is a WS session in the room that makes a change once the copy is in."""
    name = code.lower().replace("_", "-")
    before = oh.ops_rows(srv)
    gui = m7h.Gui(f"o03-{name}", port, dwg=dwg, user="u1").start()
    R.check("setup", gui.wait_ready(180) is not None, f"{code}：实例打开副本 {dwg.name}")
    held = oh.held_apart(gui, "refused", 90)
    status = held or gui.status()
    welcome = (m7h.frames(gui.err_text(), "<-", "welcome") or [{}])[-1]
    lines = oh.told_with(gui, code, oh.GET_LATEST)
    R.dump(f"o03-{name}-status.json", {"status": status, "welcome": welcome, "told": oh.told(gui)})
    R.check("OS-08", held is not None and status.get("align_code") == code
            and ((welcome.get("align_refused") or {}).get("code")) == code and bool(lines),
            f"{code}：状态 align={status.get('align')} align_code={status.get('align_code')}，"
            f"welcome.align_refused={welcome.get('align_refused')}；命令行：{lines[-1] if lines else '（没有）'}")
    resp = oh.set_prop(gui, MATERIAL, OWN[2])
    queued_now = gui.wait_status(lambda s: s.get("queued_ops") == 2, 10) or gui.status()
    live = oh.ws_write(room, current(srv), f"op-o03-live-{name}", PRESSURE, "8.8MPa")
    m7h.wait_until(lambda: any(f.get("op_id") == f"op-o03-live-{name}"
                               for f in m7h.frames(gui.err_text(), "<-", "op_applied")), 5)
    time.sleep(1.5)
    after = oh.ops_rows(srv)
    added = [r["op_id"] for r in after if r not in before]
    sent = oh.sent_ops(gui)
    own = (oh.value(gui, PRESSURE), oh.value(gui, TEMPERATURE), oh.value(gui, MATERIAL))
    heard = [f.get("revision") for f in m7h.frames(gui.err_text(), "<-", "op_applied")]
    R.check("OS-08", yh.ok(resp) and queued_now.get("queued_ops") == 2 and not sent
            and added == [f"op-o03-live-{name}"] and (live or {}).get("type") == "op_applied" and own == OWN,
            f"{code}：本地再改一步后排队 {queued_now.get('queued_ops')} 步，发出的 op {[f.get('op_id') for f in sent]}；"
            f"服务器日志新增 {added}（只应有 WS 会话的那一步）；图里的设计压力、设计温度、材质 {own}"
            f"（应为本图自己的 {OWN}），收到的 op_applied 修订 {heard}")
    saved = oh.save(gui)
    gone, how = oh.quit_gui(gui)
    R.check("setup", saved[0] and gone, f"{code}：保存（{saved[1]}）、退出（{how}）")
    on_file = oh.read_file(f"o03-{name}-file", port + 1, dwg)
    R.dump(f"o03-{name}-file.json", on_file)
    claim = oh.claim_of(on_file)
    in_file = oh.file_queue(on_file)
    R.check("OS-08", claim == ("PID-1101", repo, 1, None) and len(in_file) == 2 and in_file[0][0] == queued
            and all(user == "u1" for _, user in in_file),
            f"{code}：保存后的文件里同步记录 {claim}（应仍是 PID-1101、原来的库标识、修订 1），队列 {in_file}")


try:
    if not m7h.SEED_DWG.exists():
        R.check("setup", False, f"种子图不存在：{m7h.SEED_DWG}")
        raise SystemExit(1)
    dwg_a = m7h.fresh_copy("o03-PID-1101-a.dwg", m7h.SEED_DWG)
    srv = m7h.Server(3240, tag="o03-srv").start(fresh=True)
    R.check("setup", srv.wait_health(), f"服务器 /health = {srv.health()}")
    srv.kill()
    rev0, rev1 = m7h.WORK / "o03-rev0.db", m7h.WORK / "o03-rev1.db"
    oh.copy_db(srv.db, rev0)
    srv.start(fresh=False)
    R.check("setup", srv.wait_health(), "把修订 0 的库另存一份后，服务器用同一个库重新起来")
    u1 = m7h.Gui("o03-u1", 3210, dwg=dwg_a, user="u1").start()
    R.check("setup", u1.wait_ready(180) is not None, "u1 就绪")
    resp = None
    if u1.wait_status(lambda s: s.get("connected") and s.get("session_id"), 30):
        resp = oh.set_prop(u1, PRESSURE, OWN[0])
    at1 = u1.wait_status(lambda s: s.get("base_revision") == 1 and s.get("queued_ops") == 0, 15)
    repo = u1.status().get("repo_id")
    R.check("setup", resp is not None and yh.ok(resp) and at1 is not None and bool(repo),
            f"u1 加入并在线改设计压力为 {OWN[0]}：到修订 1，库标识 {repo}")
    srv.kill()
    oh.copy_db(srv.db, rev1)
    R.check("setup", oh.away(u1), "杀掉服务器，把修订 1 的库另存一份；u1 未连接")
    resp = oh.set_prop(u1, TEMPERATURE, OWN[1])
    saved = oh.save(u1)
    exported = oh.export(u1, m7h.WORK / "o03-queue.json")
    queued = (oh.queued_ids(exported) or [None])[0]
    gone, how = oh.quit_gui(u1)
    R.check("setup", yh.ok(resp) and saved[0] and len(oh.queued_ids(exported)) == 1 and gone,
            f"u1 离线改设计温度为 {OWN[1]}，排队 {oh.queued_ids(exported)}，保存、退出（{how}）")
    copies = {code: m7h.fresh_copy(f"o03-{code.lower().replace('_', '-')}.dwg", dwg_a)
              for code in ("REPO_MISMATCH", "REVISION_AHEAD", "REPLAY_GAP")}

    # ── REPO_MISMATCH，和 OS-09 ─────────────────────────────────────────────────────────
    other = m7h.Server(3240, tag="o03-other").start(fresh=True)
    R.check("setup", other.wait_health(), "服务器换一个新库起来")
    room, welcome = oh.ws_join(other, "o03-room-repo", "u9")
    first = oh.ws_write(room, (welcome or {}).get("base_revision") or 0, "op-o03-room-1", PRESSURE, "9.9MPa")
    R.check("setup", (first or {}).get("type") == "op_applied" and (welcome or {}).get("repo_id") not in (None, repo),
            f"新库的库标识与图里的不同；房间里先有一条修改（修订 {(first or {}).get('revision')}）")
    refused("REPO_MISMATCH", other, copies["REPO_MISMATCH"], 3211, room, repo, queued)
    observer, _w = oh.ws_join(other, "o03-observer", "u8")
    forced, wf = oh.ws_join(other, "o03-forced", "u7", repo_id="r-o03-nowhere", since=0)
    refusal = (wf or {}).get("align_refused") or {}
    rows0, value0 = oh.ops_rows(other), oh.server_value(other, PRESSURE)
    answer = oh.ws_write(forced, (wf or {}).get("base_revision") or 0, "op-o03-forced-1", PRESSURE, "6.6MPa")
    time.sleep(1.5)
    rows1, value1 = oh.ops_rows(other), oh.server_value(other, PRESSURE)
    spread = [m for m in observer.incoming("op_applied") if m.get("op_id") == "op-o03-forced-1"]
    R.dump("o03-forced.json", {"welcome": wf, "answer": answer, "ops_before": rows0, "ops_after": rows1})
    R.check("OS-09", refusal.get("code") == "REPO_MISMATCH" and (answer or {}).get("type") == "error"
            and answer.get("code") == "NOT_ALIGNED" and answer.get("op_id") == "op-o03-forced-1" and not answer.get("rejected"),
            f"hello 报别的库标识：welcome.align_refused={refusal}；硬发的 op 得到 {answer}")
    R.check("OS-09", rows1 == rows0 and value1 == value0 and not spread,
            f"房间里没有这一步：操作日志 {len(rows0)} → {len(rows1)} 行，设计压力 {value0} → {value1}，"
            f"在场的会话收到它的广播 {len(spread)} 条")
    for ws in (room, observer, forced):
        ws.close()
    other.kill()

    # ── REVISION_AHEAD ────────────────────────────────────────────────────────────────────
    oh.copy_db(rev0, srv.db)
    srv.start(fresh=False)
    room, welcome = oh.ws_join(srv, "o03-room-ahead", "u9")
    R.check("setup", srv.wait_health() and (welcome or {}).get("repo_id") == repo and (welcome or {}).get("base_revision") == 0,
            f"服务器换回修订 0 的库：库标识相同，房间修订 {(welcome or {}).get('base_revision')}")
    refused("REVISION_AHEAD", srv, copies["REVISION_AHEAD"], 3213, room, repo, queued)
    room.close()
    srv.kill()

    # ── REPLAY_GAP ────────────────────────────────────────────────────────────────────────
    oh.copy_db(rev1, srv.db)
    srv.start(fresh=False)
    R.check("setup", srv.wait_health(), "服务器换回修订 1 的库")
    room, welcome = oh.ws_join(srv, "o03-room-gap", "u9")
    second = oh.ws_write(room, 1, "op-o03-room-2", PRESSURE, "9.9MPa")
    third = oh.ws_write(room, 2, "op-o03-room-3", TEMPERATURE, "99℃")
    room.close()
    srv.kill()
    dropped = oh.drop_op(srv.db, 2)
    srv.start(fresh=False)
    R.check("setup", srv.wait_health() and (second or {}).get("revision") == 2 and (third or {}).get("revision") == 3
            and dropped == 1 and [r["revision"] for r in oh.ops_rows(srv)] == [1, 3],
            f"房间往前走了两步（修订 {(second or {}).get('revision')}、{(third or {}).get('revision')}），停下服务器删掉"
            f"修订 2 的日志（{dropped} 行）再起来：日志里的修订 {[r['revision'] for r in oh.ops_rows(srv)]}")
    room, welcome = oh.ws_join(srv, "o03-room-gap-2", "u9")
    refused("REPLAY_GAP", srv, copies["REPLAY_GAP"], 3215, room, repo, queued)
    room.close()
except SystemExit:
    raise
except Exception as error:  # noqa: BLE001
    import traceback
    R.check("script", False, f"脚本异常: {error!r}\n{traceback.format_exc()}")
finally:
    m7h.cleanup()
    ok = R.summary()
sys.exit(0 if ok else 1)
