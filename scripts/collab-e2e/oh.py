# -*- coding: utf-8 -*-
"""Helpers shared by the offline scenarios (o01 – o04), on top of m7h and yh.

What `SPEC-collab-offline-sync.md` §8 has these scenarios look at: the status
interface (`align`, `align_code`, `align_message`, `unsynced_count`, and
`queued_ops`, `diverged` from before), the refusals the status lists, the
server's op log, and the three records a saved drawing carries. The records
are read from the file as it was saved, by an instance with no collaboration
that opens it (`read_file`): the instance that saved them writes them into
the saved copy only, never into the drawing it goes on editing.
"""
import json
import shutil
import sqlite3
from pathlib import Path

import m7h

OBJ = "P-1001"
DRAWING = "PID-1101"
COLLAB, QUEUE, UNSYNCED = "PID_AUTHOR_COLLAB", "PID_AUTHOR_COLLAB_QUEUE", "PID_AUTHOR_COLLAB_UNSYNCED"
RECORDS = (COLLAB, QUEUE, UNSYNCED)
GET_LATEST = "需要取得最新图纸"


# ── the drawing in a GUI ────────────────────────────────────────────────────────────
def props(gui, obj=OBJ):
    """The properties `gui`'s drawing holds for the object tagged `obj`; None when it has no such object."""
    handle = gui.tagged().get(obj)
    if not handle:
        return None
    try:
        return json.loads(gui.author(handle).get("props") or "{}")
    except ValueError:
        return {}


def value(gui, field, obj=OBJ):
    return (props(gui, obj) or {}).get(field)


def set_prop(gui, field, text, obj=OBJ):
    return gui.run(f"PIDSETPROP {obj} {field} {text}")


def save(gui):
    """POST /save of the drawing; (saved, http status, body)."""
    st, body = gui.api("POST", "/save", {"document_id": gui.doc_id()})
    return st == 200 and (body or {}).get("ok") is True, st, body


def told(gui):
    """The command line's lines, oldest first."""
    st, body = gui.op("history")
    return [entry.get("text") or "" for entry in (body or {}).get("entries") or []]


def told_with(gui, *needles):
    return [line for line in told(gui) if all(n in line for n in needles)]


def export(gui, path):
    """`PIDCOLLAB EXPORT <path>` (§4.4): the file it wrote, or None."""
    path = Path(path)
    if path.exists():
        path.unlink()
    gui.run(f"PIDCOLLAB EXPORT {path}")
    m7h.wait_until(path.exists, 10)
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None


def queued_ids(exported):
    """The op ids of the `queued` entries of an export, in its order."""
    return [e.get("op_id") for e in (exported or {}).get("entries") or [] if e.get("state") == "queued"]


def run_of(op_id):
    """The run part of an op id `op-<run>-<n>` (§4.2)."""
    text = str(op_id or "")
    return text[3:].rsplit("-", 1)[0] if text.startswith("op-") and "-" in text[3:] else None


# ── the session ───────────────────────────────────────────────────────────────────────
def away(gui, seconds=20):
    return gui.wait_status(lambda s: s.get("connected") is False and not s.get("session_id"), seconds) is not None


def joined(gui, seconds=90, also=None):
    """Wait until `gui` is in a session with nothing left to send and no replay
    going on (and `also(status)` holds); that status, or None."""
    return gui.wait_status(
        lambda s: s.get("connected") and s.get("session_id") and s.get("queued_ops") == 0
        and s.get("catching_up") is None and (also is None or also(s)), seconds)


def held_apart(gui, align, seconds=90):
    """Wait until `gui` is in a session and its status says `align`; that status, or None."""
    return gui.wait_status(lambda s: s.get("connected") and s.get("session_id") and s.get("align") == align, seconds)


def wire(gui):
    """Every frame `gui` sent (`->`) and received (`<-`) since its last start, in order."""
    out = []
    for line in m7h.this_run(gui.err_text()).splitlines():
        for direction in ("->", "<-"):
            marker = f"帧 {direction} "
            at = line.find(marker)
            if at < 0:
                continue
            try:
                out.append((direction, json.loads(line[at + len(marker):])))
            except ValueError:
                pass
            break
    return out


def sent_ops(gui):
    return m7h.frames(gui.err_text(), "->", "op")


def hellos(gui):
    return m7h.frames(gui.err_text(), "->", "hello")


def claims_nothing(hello):
    """A `hello` that names no repository, no revision and no tags."""
    return not any(key in hello for key in ("repo_id", "since", "existing_tags"))


def quit_gui(gui):
    """Quit the way a person does: QUIT, and 退出 when the question about steps
    the room has not taken comes up (§5.6). (gone, what happened)."""
    try:
        gui.run("QUIT")
    except Exception:  # noqa: BLE001 — it may be gone before it answers
        pass
    if gui.wait_exit(4):
        return True, "QUIT 后退出，没有提示"
    try:
        st, body = gui.api("POST", "/run", {"document_id": gui.doc_id(), "op": "action", "name": "dialog_ok"})
        answered = f"答了提示（dialog_ok -> {st} {json.dumps(body, ensure_ascii=False)[:120]}）"
    except Exception as error:  # noqa: BLE001
        answered = f"dialog_ok 没有答复（{type(error).__name__}: {error}）"
    if gui.wait_exit(20):
        return True, f"QUIT 后{answered}，退出"
    gui.kill()
    return False, f"QUIT 后{answered}，20 秒内没有退出，结束了进程"


def drop_stale(dwg):
    """What an instance that had `dwg` open may have left next to it (lock and
    recovery files), the way `m7h.fresh_copy` drops them."""
    dwg = Path(dwg)
    for stale in list(dwg.parent.glob(f"{dwg.name}.ocs*")) + list(dwg.parent.glob(f".{dwg.name}*")):
        try:
            stale.unlink()
        except OSError:
            pass


def reopen(gui, seconds=180):
    """Start `gui` again on its drawing; whether it became ready."""
    drop_stale(gui.dwg)
    gui.start()
    return gui.wait_ready(seconds) is not None


# ── the file ───────────────────────────────────────────────────────────────────────────
def records(gui):
    """The three records of the drawing open in `gui`, {key: [item, ...]} with
    every item parsed; only the keys that are there. The encoding is
    `src/pid/graph_data.rs`'s: `kind=graph_data`, `key=<key>`, then one JSON
    item per entry, an item over 2000 bytes cut into `part=<n>:<piece>`
    entries from `part=0`."""
    st, body = gui.api("GET", "/records?collection=objects&limit=10000")
    out = {}
    for rec in (body or {}).get("records") or []:
        found = rec.get("properties") or {}
        if rec.get("type") != "XRecord" or found.get("name") not in RECORDS:
            continue
        strings = [e["value"]["String"] for e in found.get("entries") or []
                   if isinstance(e.get("value"), dict) and "String" in e["value"]]
        items, piece = [], None
        for s in strings:
            if s == "kind=graph_data" or s.startswith("key="):
                continue
            if s.startswith("part=") and ":" in s:
                number, chunk = s[len("part="):].split(":", 1)
                if number == "0" and piece is not None:
                    items.append(piece)
                    piece = None
                piece = (piece or "") + chunk
                continue
            if piece is not None:
                items.append(piece)
                piece = None
            items.append(s)
        if piece is not None:
            items.append(piece)
        parsed = []
        for item in items:
            try:
                parsed.append(json.loads(item))
            except ValueError:
                parsed.append({"<unparsed>": item})
        out[found["name"]] = parsed
    return out


def read_file(name, port, dwg):
    """What the file `dwg` holds, as an instance with no collaboration reads
    it: its three records ({key: [item, ...]}) and, under "props", the
    properties of OBJ. The instance quits after."""
    drop_stale(dwg)
    gui = m7h.Gui(name, port, dwg=dwg, collab=False).start()
    try:
        if gui.wait_ready(180) is None:
            raise RuntimeError(f"{name} 没有就绪")
        found = records(gui)
        found["props"] = props(gui)
        return found
    finally:
        if gui.running():
            quit_gui(gui)


def claim_of(found):
    """(drawing_id, repo_id, revision, diverged) of a file's PID_AUTHOR_COLLAB, or None."""
    items = (found or {}).get(COLLAB) or []
    if not items:
        return None
    item = items[0]
    return item.get("drawing_id"), item.get("repo_id"), item.get("revision"), item.get("diverged")


def file_queue(found):
    """(op_id, user_id) of every entry of a file's queue, in its order."""
    return [(e.get("op_id"), e.get("user_id")) for e in (found or {}).get(QUEUE) or []]


# ── the server ──────────────────────────────────────────────────────────────────────
def server_value(srv, field, obj=OBJ):
    status, body = srv.http("GET", f"/drawing/{DRAWING}/objects?object_ref={obj}")
    items = (body or {}).get("items") or []
    return ((items[0].get("fields") or {}).get(field) or {}).get("value") if items else None


def ops_rows(srv):
    return srv.sql("select revision, op_id, user_id, base_revision from ops where drawing_id = ? order by revision",
                   (DRAWING,))


def copy_db(source, target):
    """Copy a stopped server's database, its WAL files with it."""
    for suffix in ("", "-wal", "-shm"):
        a, b = Path(str(source) + suffix), Path(str(target) + suffix)
        if b.exists():
            b.unlink()
        if a.exists():
            shutil.copyfile(a, b)


def drop_op(db, revision):
    """Delete the op log's row for `revision` from a stopped server's database;
    how many rows went."""
    con = sqlite3.connect(str(db), timeout=10)
    try:
        gone = con.execute("delete from ops where drawing_id = ? and revision = ?", (DRAWING, revision)).rowcount
        con.commit()
        return gone
    finally:
        con.close()


def ws_join(srv, name, user, repo_id=None, since=None):
    """A WS session of its own on the drawing, kept alive: (ws, welcome)."""
    ws = m7h.Ws(f"ws://127.0.0.1:{srv.port}/ws", name)
    hello = {"type": "hello", "user_id": user, "drawing_id": DRAWING, "mode": "协同编辑", "client_version": name}
    if repo_id is not None:
        hello["repo_id"] = repo_id
    if since is not None:
        hello["since"] = since
    ws.send(hello)
    welcome = ws.wait_for(lambda m: m.get("type") in ("welcome", "error"), 10)
    ws.session_id = (welcome or {}).get("session_id")
    if ws.session_id:
        ws.keep_alive(10)
    return ws, welcome


def ws_write(ws, base, op_id, field, text, obj=OBJ, seconds=10):
    """Send one set_property op; the answer that names it (op_applied, error, conflict), or None."""
    ws.send({"type": "op", "session_id": ws.session_id, "base_revision": base, "op_id": op_id,
             "payload": {"kind": "set_property", "object_ref": obj, "field": field, "value": text}})
    return ws.wait_for(lambda m: m.get("type") in ("op_applied", "error", "conflict") and m.get("op_id") == op_id,
                       seconds)
