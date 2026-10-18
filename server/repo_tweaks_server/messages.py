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

    def line(f, has_meta):
        notes = []
        if has_meta:
            notes.append(".meta も")
        if kind == "force_unlock":
            if len(owners) > 1:
                notes.append(f"{_plain(f.get('owner') or '（不明）')} さんのロック")
            if f.get("locked_at"):
                notes.append(f"{jst(f['locked_at'])} から")
        return _item(f["path"], notes)

    return _compose(header, [line(f, m) for f, m in rows])


def _item(path, notes):
    return f"・ {_code(path)}" + (f"（{'、'.join(notes)}）" if notes else "")


def _compose(header, lines):
    """Header plus as many lines as fit (at most MAX_LISTED_FILES), then "ほか N 件" for the rest"""
    out = [header]
    length = len(header)
    for line in lines[:MAX_LISTED_FILES]:
        # Leave room for the "ほか N 件" line
        if length + 1 + len(line) > MAX_MESSAGE_CHARS - 20:
            break
        out.append(line)
        length += 1 + len(line)
    shown = len(out) - 1
    if len(lines) > shown:
        out.append(f"ほか {len(lines) - shown} 件")
    return "\n".join(out)


# --- Periodic check (reconcile.py): locks taken or released without the CLI ---


def found_message(remote, locks):
    """CLI を通らずに取られたロック（照合で見つけたもの）"""
    rows = _group_meta(locks)
    lines = [_item(f["path"], ([".meta も"] if m else []) +
                   [f"{_plain(f.get('owner') or '（不明）')} さん", f"{jst(f.get('locked_at'))} から"]) for f, m in rows]
    return _compose(f"🔍 ロックを見つけました（{_plain(_label(remote))}）", lines)


def released_message(remote, locks):
    """CLI を通らずに外れたロック（照合で消えていたもの）"""
    rows = _group_meta(locks)
    lines = [_item(f["path"], ([".meta も"] if m else []) +
                   [f"{_plain(f.get('owner') or '（不明）')} さんのロック", f"{jst(f.get('locked_at'))} から"])
             for f, m in rows]
    return _compose(f"🔓 ロックが外れました（{_plain(_label(remote))}）", lines)


def stale_message(items, days, now, failing=()):
    """items: [(remote, lock)] で、days 日以上ロックされたままのもの。failing: 照合が失敗中で含めていないもの"""
    def age(lock):
        try:
            seconds = (now - datetime.fromisoformat(lock["locked_at"].replace("Z", "+00:00"))).total_seconds()
        except (KeyError, ValueError):
            return "—"
        hours = max(0, int(seconds // 3600))
        d, h = divmod(hours, 24)
        return f"{d}日{h}時間" if h else f"{d}日"

    lines = [_item(lock["path"], [_plain(_label(remote)), f"{_plain(lock.get('owner') or '（不明）')} さん",
                                  f"{jst(lock.get('locked_at'))} から", age(lock)])
             for remote, lock in items]
    text = _compose(f"⏰ {days}日以上ロックされたままのものが {len(items)} 件あります", lines)
    if failing:
        names = "、".join(_plain(_label(r)) for r in failing)
        text += f"\n※ 照合に失敗しているため含めていません: {names}"
    return text


def check_failed_message(remote, reason):
    return f"⚠️ ロックの照合ができませんでした（{_plain(_label(remote))}）: {_plain(reason)}"


def check_recovered_message(remote):
    return f"✅ ロックの照合が直りました（{_plain(_label(remote))}）"


def _label(remote):
    """github.com/example-org/game -> example-org/game"""
    parts = (remote or "").split("/")
    return "/".join(parts[1:]) if len(parts) >= 3 else (remote or "（リポジトリ不明）")


def _repo_label(event):
    """The remote without its host; falls back to the folder name"""
    if event.get("remote"):
        return _label(event["remote"])
    return event.get("repo") or "（リポジトリ不明）"
