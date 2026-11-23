# -*- coding: utf-8 -*-
"""OS-19（SPEC-collab-offline-sync.md §6、§7、§8）：服务器的库打不开时不接受会话；用 --new-repo-id 启动后旧图纸得到 REPO_MISMATCH。

准备：服务器（3240，新库），u1（3210）打开种子图 A 加入，在线改设计压力为 1.0MPa（修订 1，记下
库标识）。杀掉服务器，u1 离线改设计温度为 150℃（排队一步），保存、退出。

库打不开：另起一个服务器，--db 指向一个目录。进程照常在跑，GET /health 和 POST /session 都是 503，标准
错误上有「数据库初始化失败」和「不接受会话」两行。一个 WS 连接发 hello：收到 error STORE_UNAVAILABLE，接着
是关闭帧，没有 welcome。u1 打开 A：没有进会话，一步都没发，队列还是那一步，命令行说服务器拒绝了请求
（STORE_UNAVAILABLE）；服务器输出里有拒绝 u1 的 hello 的一行。

换库标识：用原来的库带 --new-repo-id 启动：输出「库标识已换成 ……」，新标识与 A 里记的不同，从头加入的 WS
会话在 welcome 里拿到的就是它。u1 重开 A（hello 报的是旧标识）：状态是对不上（REPO_MISMATCH），welcome 里
带着这个码，命令行说「需要取得最新图纸」，一步都没发，服务器日志不变，队列还在。再不带参数启动：标识还是新的，修订 1 那一步还在。

证据：logs/o08-*.json、logs/gui-o08-*.err.log、logs/o08-*.out.log、logs/o08-*.err.log、logs/ws-o08-*.jsonl。
"""
import sys
import time

import m7h
import oh
import yh

R = m7h.Results("o08_server_store")
PRESSURE, TEMPERATURE = "设计压力", "设计温度"


def started(name, port, dwg, **more):
    gui = m7h.Gui(name, port, dwg=dwg, **more).start()
    R.check("setup", gui.wait_ready(180) is not None, f"{name} 打开 {dwg.name}")
    return gui


def err_text(srv):
    return m7h.this_run(srv.err.read_text(encoding="utf-8", errors="replace")) if srv.err.exists() else ""


def out_text(srv):
    return m7h.this_run(srv.log_text())


def asked(srv, method, path, body=None):
    """srv.http, or (0, the error) when nothing answers (the server is gone)."""
    try:
        return srv.http(method, path, body)
    except OSError as error:
        return 0, {"error": str(error)}


def fresh_join(srv, name):
    """A WS session that names no repository; (repo_id of its welcome, the welcome)."""
    ws, welcome = oh.ws_join(srv, name, "u2")
    ws.close()
    return (welcome or {}).get("repo_id"), welcome


try:
    if not m7h.SEED_DWG.exists():
        R.check("setup", False, f"种子图不存在：{m7h.SEED_DWG}")
        raise SystemExit(1)
    dwg_a = m7h.fresh_copy("o08-PID-1101-a.dwg", m7h.SEED_DWG)
    srv = m7h.Server(3240, tag="o08-srv").start(fresh=True)
    R.check("setup", srv.wait_health(), f"服务器 /health = {srv.health()}")
    u1 = started("o08-u1", 3210, dwg_a, user="u1")
    resp = None
    if u1.wait_status(lambda s: s.get("connected") and s.get("session_id"), 30):
        resp = oh.set_prop(u1, PRESSURE, "1.0MPa")
    at1 = u1.wait_status(lambda s: s.get("base_revision") == 1 and s.get("queued_ops") == 0, 15)
    repo = u1.status().get("repo_id")
    R.check("setup", resp is not None and yh.ok(resp) and at1 is not None and bool(repo),
            f"u1 加入并在线改设计压力为 1.0MPa：到修订 1，库标识 {repo}")
    srv.kill()
    R.check("setup", oh.away(u1), "杀掉服务器；u1 未连接")
    resp = oh.set_prop(u1, TEMPERATURE, "150℃")
    saved = oh.save(u1)
    gone, how = oh.quit_gui(u1)
    R.check("setup", yh.ok(resp) and saved[0] and gone,
            f"u1 离线改设计温度为 150℃（排队一步），保存（{saved[1]}）、退出（{how}）")
    rows = oh.ops_rows(srv)

    # ── 库打不开 ──────────────────────────────────────────────────────────────────────────
    folder = m7h.WORK / "o08-store-is-a-folder"
    folder.mkdir(exist_ok=True)
    bad = m7h.Server(3240, db=folder, tag="o08-bad").start(fresh=False, with_seed=False)
    m7h.wait_until(lambda: "不接受会话" in err_text(bad) or not bad.running(), 15)
    m7h.wait_until(lambda: bad.health()[0] != 0 or not bad.running(), 15)
    health = bad.health()
    session = asked(bad, "POST", "/session", {"user_id": "u1", "drawing_id": oh.DRAWING, "mode": "协同编辑"})
    said = [line for line in err_text(bad).splitlines() if "数据库初始化失败" in line or "不接受会话" in line]
    R.dump("o08-bad-start.json", {"health": health, "session": session, "stderr": err_text(bad)})
    R.check("OS-19", bad.running() and health[0] == 503 and session[0] == 503
            and any("数据库初始化失败" in line for line in said) and any("不接受会话" in line for line in said),
            f"--db 指向一个目录：进程{'还在' if bad.running() else '已退出'}，GET /health -> {health[0]}，"
            f"POST /session -> {session[0]}；标准错误上的两行 {said}")
    try:
        probe = m7h.Ws(f"ws://127.0.0.1:{bad.port}/ws", "o08-probe")
    except OSError as error:
        probe, got = None, [{"type": "<no connection>", "message": str(error)}]
    if probe:
        probe.send({"type": "hello", "user_id": "u2", "drawing_id": oh.DRAWING, "mode": "协同编辑",
                    "client_version": "o08-probe", "existing_tags": ["V-9001"]})
        m7h.wait_until(lambda: probe.closed, 10)
        got = probe.incoming()
        probe.close()
    kinds = [m.get("type") for m in got]
    error = next((m for m in got if m.get("type") == "error"), {})
    R.dump("o08-bad-hello.json", got)
    R.check("OS-19", error.get("code") == "STORE_UNAVAILABLE" and "welcome" not in kinds and "<close>" in kinds
            and kinds.index("<close>") > kinds.index("error"),
            f"WS 连接发 hello：收到的依次是 {kinds}，error 的码 {error.get('code')}，说明「{error.get('message')}」")
    u1 = started("o08-u1-bad", 3211, dwg_a, user="u1")
    m7h.wait_until(lambda: bool(oh.told_with(u1, "STORE_UNAVAILABLE")), 30)
    time.sleep(2)
    status = u1.status()
    told = oh.told_with(u1, "STORE_UNAVAILABLE")
    refused = [line for line in out_text(bad).splitlines() if "拒绝 hello" in line and "user=u1" in line]
    R.dump("o08-bad-u1.json", {"status": status, "told": told, "hellos": oh.hellos(u1), "server": refused})
    R.check("OS-19", bool(told) and not status.get("session_id") and status.get("queued_ops") == 1
            and not oh.sent_ops(u1) and bool(oh.hellos(u1)) and bool(refused),
            f"u1 打开 A（队列一步）：发了 {len(oh.hellos(u1))} 次 hello，没有会话（session_id={status.get('session_id')!r}），"
            f"发出的 op {len(oh.sent_ops(u1))} 条，队列 {status.get('queued_ops')} 条；命令行 {told[-1:] if told else '没有说'}；"
            f"服务器输出里拒绝 u1 的 hello {len(refused)} 行")
    gone, how = oh.quit_gui(u1)
    R.check("setup", gone, f"u1 退出（{how}）")
    bad.kill()

    # ── 换库标识 ─────────────────────────────────────────────────────────────────────────
    renew = m7h.Server(3240, db=srv.db, tag="o08-renew", extra=["--new-repo-id"]).start(fresh=False)
    R.check("setup", renew.wait_health(), "用原来的库带 --new-repo-id 启动")
    renewed = [line for line in out_text(renew).splitlines() if "库标识已换成" in line]
    new_repo, welcome = fresh_join(renew, "o08-fresh")
    R.check("OS-19", bool(renewed) and bool(new_repo) and new_repo != repo and new_repo in renewed[-1],
            f"带 --new-repo-id 启动：输出 {renewed}；从头加入的 WS 会话拿到的库标识 {new_repo}，"
            f"A 里记的是 {repo}")
    u1 = started("o08-u1-renew", 3212, dwg_a, user="u1")
    apart = oh.held_apart(u1, "refused", 60)
    time.sleep(2)
    status = apart or u1.status()
    hello = (oh.hellos(u1) or [{}])[-1]
    welcome = (m7h.frames(u1.err_text(), "<-", "welcome") or [{}])[-1]
    lines = oh.told_with(u1, "REPO_MISMATCH", oh.GET_LATEST)
    R.dump("o08-renew-u1.json", {"status": status, "hellos": oh.hellos(u1), "welcome": welcome, "told": oh.told(u1)})
    R.check("OS-19", apart is not None and status.get("align_code") == "REPO_MISMATCH"
            and (welcome.get("align_refused") or {}).get("code") == "REPO_MISMATCH" and bool(lines)
            and hello.get("repo_id") == repo and not oh.sent_ops(u1) and oh.ops_rows(renew) == rows
            and status.get("queued_ops") == 1,
            f"u1 重开 A（hello 报的库标识{'是旧的' if hello.get('repo_id') == repo else '是 ' + str(hello.get('repo_id'))}）："
            f"align={status.get('align')} align_code={status.get('align_code')}，说明「{status.get('align_message')}」，"
            f"welcome.align_refused={welcome.get('align_refused')}；命令行：{lines[-1] if lines else '（没有）'}；"
            f"发出的 op {len(oh.sent_ops(u1))} 条，服务器日志 {len(rows)} → {len(oh.ops_rows(renew))} 行，队列 {status.get('queued_ops')} 条")
    gone, how = oh.quit_gui(u1)
    R.check("setup", gone, f"u1 退出（{how}）")
    renew.kill()
    again = m7h.Server(3240, db=srv.db, tag="o08-again").start(fresh=False)
    R.check("setup", again.wait_health(), "再不带 --new-repo-id 启动")
    kept_repo, _welcome = fresh_join(again, "o08-fresh-again")
    R.check("OS-19", kept_repo == new_repo and oh.ops_rows(again) == rows and not any(
        "库标识已换成" in line for line in out_text(again).splitlines()),
            f"再不带参数启动：库标识 {kept_repo}（{'还是新的' if kept_repo == new_repo else '变了'}），"
            f"操作日志 {[(r['revision'], r['op_id']) for r in oh.ops_rows(again)]}")
except SystemExit:
    raise
except Exception as error:  # noqa: BLE001
    import traceback
    R.check("script", False, f"脚本异常: {error!r}\n{traceback.format_exc()}")
finally:
    m7h.cleanup()
    ok = R.summary()
sys.exit(0 if ok else 1)
