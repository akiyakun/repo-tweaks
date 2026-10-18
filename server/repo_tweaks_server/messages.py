"""Discord に投稿する文面（ロック・解除の通知）

1 回の操作を 1 つのメッセージにする。.meta は元のファイルの行にまとめる。
ファイル名・人の名前は利用者が決める文字列なので、Discord の書式として解釈されないようにする
（メンションは notify.py の allowed_mentions で止める）。
"""

from datetime import datetime, timedelta, timezone

JST = timezone(timedelta(hours=9))
LOCK_EVENTS = ("lock", "unlock", "force_unlock")
MAX_LISTED_FILES = 20
# notify.py cuts at 1900; stay below it so the list ends cleanly with "ほか N 件"
MAX_MESSAGE_CHARS = 1800
# Long paths keep their end (the file name matters most)
MAX_SHOWN_PATH = 150
META = ".meta"


def jst(value):
    """「2026-10-18T04:05:54Z」を「10/18 13:05」にする"""
    if not value:
        return "—"
    try:
        return datetime.fromisoformat(value.replace("Z", "+00:00")).astimezone(JST).strftime("%m/%d %H:%M")
    except ValueError:
        return value


def _plain(text):
    """Escape Discord markdown in names (mentions are blocked separately)"""
    return "".join("\\" + c if c in "\\*_~`|>[]()#" else c for c in text)


def _code(path):
    # Inside a code span nothing is formatted; backticks themselves cannot appear in one
    if len(path) > MAX_SHOWN_PATH:
        path = "…" + path[-(MAX_SHOWN_PATH - 1):]
    return "`" + path.replace("`", "'") + "`"


def _group_meta(files):
    """Fold "X.meta" into "X" when both are present and held by the same person: [(file, has_meta), ...]"""
    by_path = {f["path"]: f for f in files}

    def same_owner(a, b):
        return a.get("owner") == b.get("owner")

    out = []
    for f in files:
        path = f["path"]
        base = by_path.get(path[: -len(META)]) if path.endswith(META) else None
        if base is not None and same_owner(base, f):
            continue
        meta = by_path.get(path + META)
        out.append((f, meta is not None and same_owner(f, meta)))
    return out


def lock_message(event):
    """ロック・解除の通知の文面。対象外の通知なら None"""
    kind = event.get("event")
    files = event.get("files") or []
    if kind not in LOCK_EVENTS or not files:
        return None
    who = _plain(event.get("user") or "（名前なし）")
    repo = _plain(_repo_label(event))
    rows = _group_meta(files)

    if kind == "lock":
        header = f"🔒 {who} がロックしました（{repo}）"
    elif kind == "unlock":
        header = f"🔓 {who} が解除しました（{repo}）"
    else:
        owners = sorted({f.get("owner") or "（不明）" for f, _ in rows})
        whose = f" {_plain(owners[0])} さんの" if len(owners) == 1 else "他人の"
        header = f"⚠️ {who} が{whose}ロックを強制解除しました（{repo}）"

    lines = [header]
    length = len(header)
    shown = 0
    for f, has_meta in rows[:MAX_LISTED_FILES]:
        notes = []
        if has_meta:
            notes.append(".meta も")
        if kind == "force_unlock":
            if len(owners) > 1:
                notes.append(f"{_plain(f.get('owner') or '（不明）')} さんのロック")
            if f.get("locked_at"):
                notes.append(f"{jst(f['locked_at'])} から")
        line = f"・ {_code(f['path'])}" + (f"（{'、'.join(notes)}）" if notes else "")
        # Leave room for the "ほか N 件" line
        if length + 1 + len(line) > MAX_MESSAGE_CHARS - 20:
            break
        lines.append(line)
        length += 1 + len(line)
        shown += 1
    if len(rows) > shown:
        lines.append(f"ほか {len(rows) - shown} 件")
    return "\n".join(lines)


def _repo_label(event):
    """"github.com/example-org/game" -> "example-org/game"; falls back to the folder name"""
    remote = event.get("remote") or ""
    parts = remote.split("/")
    if len(parts) >= 3:
        return "/".join(parts[1:])
    return remote or event.get("repo") or "（リポジトリ不明）"
