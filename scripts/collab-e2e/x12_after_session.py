# -*- coding: utf-8 -*-
"""VAL-CROSS-012：协同会话结束后，本地文件可独立打开，没有协同残留。

u1（3210）与 u2（3211）各做一次编辑并都收到 op_applied；u1 保存并关闭文档，
两个实例正常退出。然后不带 --collab 起一个新实例（APPDATA 是新的空目录，
没有协同配置）打开 u1 保存的文件。

证据：logs/x12-*.json（新实例的 ready、entities、状态、xdata、三个
XRecord、sessions 表）、logs/gui-x12-*.err.log、shots/cross-final.png。
"""
import json
import shutil
import sys
import time

import m7h

R = m7h.Results("x12_after_session")
FIELD = "设计压力"
RECORDS = ("PID_AUTHOR_TYPE_LIB", "PID_AUTHOR_TAG_RULES", "PID_AUTHOR_TOPOLOGY")
# M3 的基线图：五个带 PID_AUTHOR 的实例，模型空间扩展字典里有图级 XRecord
# （33e0a28e5 的生成器写出的；种子图里没有这些 XRecord）。
BASELINE = m7h.WORK / "PID-1101-m3.dwg"
PROPS = '{"设计压力":"%s","设计温度":"120C","材质":"碳钢"}'
# 用户目录是空的时候，程序一启动就问的几件事（设为默认程序、捐助、显卡提示，见 src/app/startup.rs）。
# 没进过协同的程序第一次启动也问，不是协同的残留。
FIRST_LAUNCH = ("AssocPrompt", "DonationPrompt", "GpuWarning")


def graph_records(gui):
    """模型空间扩展字典里的图级 XRecord：{key: {handle, owner, strings}}。"""
    status, body = gui.api("GET", "/records?collection=objects&limit=10000")
    out = {}
    for rec in (body or {}).get("records") or []:
        if rec.get("type") != "XRecord":
            continue
        props = rec.get("properties") or {}
        strings = [e["value"]["String"] for e in props.get("entries") or []
                   if isinstance(e.get("value"), dict) and "String" in e["value"]]
        key = props.get("name") if props.get("name") in RECORDS else None
        if not key:
            continue
        # 条目的编码见 src/pid/graph_data.rs：前两条是 kind=graph_data 与 key=<键>，
        # 其余每条是一项 JSON；超过 2000 字节的一项切成 part=<序号>:<片段> 几条。
        items, pending = [], ""
        for s in strings:
            if s == "kind=graph_data" or s.startswith("key="):
                continue
            if s.startswith("part=") and ":" in s:
                pending += s.split(":", 1)[1]
                continue
            if pending:
                items.append(pending)
                pending = ""
            items.append(s)
        if pending:
            items.append(pending)
        out[key] = {"handle": rec.get("handle"), "owner": props.get("owner"), "name": props.get("name"),
                    "headers": [s for s in strings if s == "kind=graph_data" or s.startswith("key=")],
                    "strings": items, "entries": props.get("entries")}
    return status, out


def quit_gui(gui, name):
    try:
        status, body = gui.run("QUIT")
        R.note(f"{name} POST /api/v1/run QUIT -> {status} {json.dumps(body, ensure_ascii=False)[:160]}")
    except Exception as error:  # noqa: BLE001
        R.note(f"{name} QUIT 的响应没有读到（进程已退出）: {type(error).__name__}")
    return gui.wait_exit(15)


try:
    if not BASELINE.exists():
        R.check("setup", False, f"M3 基线图不存在：{BASELINE}")
        raise SystemExit(1)
    dwg_a = m7h.fresh_copy("x12-PID-1101-a.dwg", BASELINE)
    dwg_b = m7h.fresh_copy("x12-PID-1101-b.dwg", BASELINE)
    srv = m7h.Server(3240, tag="x12-srv").start(fresh=True)
    R.check("setup", srv.wait_health(), f"服务器 /health = {srv.health()}")

    u1 = m7h.Gui("x12-u1", 3210, dwg=dwg_a, user="u1").start()
    u2 = m7h.Gui("x12-u2", 3211, dwg=dwg_b, user="u2").start()
    R.check("setup", u1.wait_ready(180) is not None and u2.wait_ready(180) is not None, "两个 GUI 就绪")
    s1 = u1.wait_status(lambda s: s.get("connected") and s.get("session_id"), 30)
    s2 = u2.wait_status(lambda s: s.get("connected") and s.get("session_id"), 30)
    R.check("setup", s1 is not None and s2 is not None, "u1、u2 均已连接")
    sid1, sid2 = s1["session_id"], s2["session_id"]
    doc1, doc2 = u1.doc_id(), u2.doc_id()
    h1, h2 = u1.tagged().get("P-1001"), u2.tagged().get("P-1001")
    R.check("setup", bool(h1) and h1 == h2, f"两份图上 P-1001 的句柄: u1={h1} u2={h2}")

    # ── 一轮协同：各一次编辑，都收到 op_applied ───────────────────────────
    st, run1 = u1.run("PIDTAG P-1001 P-1009", doc1)
    m7h.wait_until(lambda: u2.author(h2).get("tag") == "P-1009", 20)
    st2, put2 = u2.api("PUT", f"/entities/{h2}/xdata/PID_AUTHOR", [{"code": 1000, "value": "props=" + PROPS % "2.5MPa"}])
    m7h.wait_until(lambda: "2.5MPa" in (u1.author(h1).get("props") or ""), 20)
    f1 = m7h.frames(u1.err_text(), "<-", "op_applied")
    f2 = m7h.frames(u2.err_text(), "<-", "op_applied")
    R.dump("x12-op-applied-frames.json", {"u1": f1, "u2": f2})
    R.check("VAL-CROSS-012", st == 200 and (run1 or {}).get("ok") is True and st2 == 200 and (put2 or {}).get("ok") is True
            and [f.get("revision") for f in f1] == [1, 2] and [f.get("revision") for f in f2] == [1, 2],
            f"u1 改位号、u2 改属性，两边各收到两条 op_applied（修订 {[f.get('revision') for f in f1]} / {[f.get('revision') for f in f2]}）")
    a1 = u1.author(h1)
    R.check("VAL-CROSS-012", a1.get("tag") == "P-1009" and "2.5MPa" in (a1.get("props") or ""),
            f"保存之前 u1 的图: tag={a1.get('tag')!r} props={a1.get('props')!r}")

    # ── u1 保存并关闭文档，两个实例退出 ──────────────────────────────────
    st, saved = u1.api("POST", "/save", {"document_id": doc1})
    R.dump("x12-u1-save.json", {"http_status": st, "body": saved})
    R.check("VAL-CROSS-012", st == 200 and (saved or {}).get("ok") is True, f"u1 保存: POST /save -> {st} ok={(saved or {}).get('ok')}")
    st, closed = u1.api("DELETE", f"/documents/{doc1}")
    R.dump("x12-u1-close.json", {"http_status": st, "body": closed})
    R.check("VAL-CROSS-012", st in (200, 204) and (closed is None or (closed or {}).get("ok") is True),
            f"u1 关闭文档: DELETE /documents/{doc1} -> {st} {json.dumps(closed, ensure_ascii=False)[:160]}")
    # u2 的图有未保存的改动；先保存，正常退出时就不会被「是否保存」拦住。
    st, saved2 = u2.api("POST", "/save", {"document_id": doc2})
    R.note(f"u2 保存: POST /save -> {st} ok={(saved2 or {}).get('ok')}")
    gone1, gone2 = quit_gui(u1, "u1"), quit_gui(u2, "u2")
    R.check("VAL-CROSS-012", gone1 and gone2,
            f"两实例退出（进程消失）: u1 {gone1}（退出码 {u1.exit_code() if gone1 else '?'}），u2 {gone2}（退出码 {u2.exit_code() if gone2 else '?'}）")
    m7h.wait_until(lambda: all(r["status"] == "失效" for r in srv.sql("select status from sessions")), 10)
    rows = srv.sql("select session_id, user_id, status from sessions order by created_at")
    R.dump("x12-sessions.json", rows)
    R.check("VAL-CROSS-012", len(rows) == 2 and {r["session_id"] for r in rows} == {sid1, sid2} and all(r["status"] == "失效" for r in rows),
            f"服务器 sessions 表中该两行: {json.dumps(rows, ensure_ascii=False)}")
    leftovers = [str(p) for g in (u1, u2) for p in g.descriptors()]
    R.check("VAL-CROSS-012", not leftovers, f"退出的两个实例没有留下自动化通道的描述文件（{len(leftovers)} 个）")

    # ── 不带 --collab 重开保存后的文件 ───────────────────────────────────
    plain = m7h.Gui("x12-plain", 3212, dwg=dwg_a, collab=False)
    if plain.appdata.exists():
        shutil.rmtree(plain.appdata)
    plain.start()
    R.note(f"新实例启动参数: {' '.join(plain.args())}")
    ready_in = plain.wait_ready(180)
    st, ready = plain.ready()
    R.dump("x12-plain-ready.json", {"http_status": st, "body": ready})
    R.check("VAL-CROSS-012", ready_in is not None and st == 200, f"新实例 GET /api/v1/ready -> {st}")
    st, ents = plain.api("GET", "/entities")
    count = len((ents or {}).get("entities") or ((ents or {}).get("result") or {}).get("entities") or [])
    R.dump("x12-plain-entities.json", {"http_status": st, "ok": (ents or {}).get("ok"), "count": count})
    R.check("VAL-CROSS-012", st == 200 and count > 0, f"新实例 GET /api/v1/entities -> {st}，实体 {count} 个")
    status = plain.status()
    R.dump("x12-plain-collab-status.json", status)
    R.check("VAL-CROSS-012", status.get("configured") is False and status.get("connected") is False
            and not status.get("last_error") and not status.get("last_apply_error"),
            f"协同未配置、无协同相关错误: configured={status.get('configured')} connected={status.get('connected')} "
            f"last_error={status.get('last_error')} last_apply_error={status.get('last_apply_error')}")
    state = plain.state()
    R.dump("x12-plain-state.json", state)
    inner = state.get("result") or {}
    # 对话框是工具窗：两次更新之间停在 tool_windows 里，modal 只在它自己的消息处理时才有值
    # （同 o06 的 modal()），所以两处一起看。
    modal = state.get("modal") or inner.get("modal")
    shown = sorted(set(state.get("tool_windows") or inner.get("tool_windows") or []) | ({modal} if modal else set()))
    others = [kind for kind in shown if kind not in FIRST_LAUNCH]
    R.check("VAL-CROSS-012", not others,
            f"没有弹出对话框（tool_windows 和 modal 里开着的 {shown}；除去首次启动的提示，剩下 {others}）")
    plain.run("PIDCOLLAB ON")
    time.sleep(1.5)
    bounds = plain.ui_bounds("pid-collab")
    st, resp, shot = plain.capture(m7h.SHOTS / "cross-final.png")
    R.check("VAL-CROSS-012", st == 200 and shot and sorted(bounds) == ["pid-collab-panel", "pid-collab-unconfigured"],
            f"截图 shots/cross-final.png；协同面板打开着，画的是未配置的提示，别的什么都没有（面板里上报位置的元素: {sorted(bounds)}）")
    tagged = plain.tagged()
    hp = tagged.get("P-1009")
    st, xd = plain.xdata(hp) if hp else (0, None)
    R.dump("x12-plain-xdata.json", xd)
    ap = plain.author(hp) if hp else {}
    R.check("VAL-CROSS-012", hp == h1 and ap.get("tag") == "P-1009" and "2.5MPa" in (ap.get("props") or "") and ap.get("type") == "EQUIPMENT",
            f"PID_AUTHOR 完整可解析: 句柄 {hp} tag={ap.get('tag')!r} type={ap.get('type')!r} props={ap.get('props')!r}")
    st, records = graph_records(plain)
    R.dump("x12-plain-records.json", records)
    parsed = {}
    for key, rec in records.items():
        try:
            parsed[key] = [json.loads(s) for s in rec["strings"]]
        except ValueError as error:
            parsed[key] = f"解析失败: {error}"
    R.note("图级 XRecord 读回: " + json.dumps({k: (records[k]["handle"], len(records[k]["strings"]), json.dumps(parsed[k], ensure_ascii=False)[:160]) for k in records}, ensure_ascii=False))
    R.check("VAL-CROSS-012", st == 200 and set(records) == set(RECORDS) and all(isinstance(v, list) and v for v in parsed.values()),
            f"三个图级 XRecord 都在且条目可解析为 JSON: {sorted(records)}（各 {[len(parsed[k]) if isinstance(parsed[k], list) else parsed[k] for k in sorted(records)]} 条）")
    R.check("VAL-CROSS-012", len(srv.sql("select session_id from sessions")) == 2,
            "新实例没有连服务器（sessions 仍是那两行）")
except SystemExit:
    raise
except Exception as error:  # noqa: BLE001
    import traceback
    R.check("script", False, f"脚本异常: {error!r}\n{traceback.format_exc()}")
finally:
    m7h.cleanup()
    ok = R.summary()
sys.exit(0 if ok else 1)
