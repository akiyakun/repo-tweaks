"""HTTP の受け口（Starlette）

- GET  /health      動いているかの確認（認証なし。Docker のヘルスチェック用）
- GET  /api/ping    トークンの確認を含めた疎通確認（CLI の `repo-tweaks ping`）
- POST /api/events  CLI・git hook からの通知を受け取る（lock / unlock / force_unlock は Discord にも投稿する）
- GET  /api/events  受け取った通知の直近の一覧（メモリにだけ置く。再起動で消える）
- /、/login など    管理画面（admin.py）

安全のための決まり:
- /api/ は Bearer トークン（REPO_TWEAKS_TOKEN）が必須。管理画面はパスワードでログインする
- 受け付けるホスト名を REPO_TWEAKS_ALLOWED_HOSTS に限る（DNS リバインディング対策）
- 本文は MAX_BODY_BYTES まで
"""

import itertools
import json
import logging
import unicodedata
from collections import deque
from datetime import datetime, timezone
from pathlib import Path

from starlette.applications import Starlette
from starlette.background import BackgroundTask
from starlette.middleware import Middleware
from starlette.middleware.trustedhost import TrustedHostMiddleware
from starlette.responses import JSONResponse
from starlette.routing import Route

from . import __version__
from .admin import SecurityHeaders, admin_routes
from .config import Settings
from .messages import LOCK_EVENTS, lock_message
from .notify import Notifier
from .security import same_token
from .settings import SettingsStore

log = logging.getLogger("repo_tweaks")

# Room for a lock event listing MAX_FILES long paths
MAX_BODY_BYTES = 64 * 1024
MAX_FIELD_LENGTH = 200
MAX_PATH_LENGTH = 1024
MAX_FILES = 200
# Events stay in memory; keep only the first few paths of each (the count is kept in file_count)
STORED_FILES = 20
# Fields a client may send; anything else is dropped
EVENT_FIELDS = ("event", "repo", "remote", "branch", "commit", "user", "sent_at")
# Control chars, surrogates, and Unicode line/paragraph separators (U+0085 is Cc, U+2028 Zl, U+2029 Zp).
# Not str.isprintable(): it also rejects U+3000, which appears in Japanese names.
REJECTED_CATEGORIES = {"Cc", "Cs", "Zl", "Zp"}


class BodyTooLarge(Exception):
    pass


async def _read_body(request):
    declared = request.headers.get("content-length")
    if declared and (not declared.isdigit() or int(declared) > MAX_BODY_BYTES):
        raise BodyTooLarge
    body = bytearray()
    async for chunk in request.stream():
        body.extend(chunk)
        if len(body) > MAX_BODY_BYTES:
            raise BodyTooLarge
    return bytes(body)


def _clean_text(value, max_length=MAX_FIELD_LENGTH):
    """短い1行の文字列だけを通す（ログを崩す改行・制御文字は不可）"""
    if not isinstance(value, str) or not value or len(value) > max_length:
        return None
    if any(unicodedata.category(c) in REJECTED_CATEGORIES for c in value):
        return None
    return value


def parse_event(data):
    """受け取った JSON から通知を作る。おかしければ ValueError（利用者向けの文面）"""
    if not isinstance(data, dict):
        raise ValueError("本文は JSON のオブジェクトにしてください")
    event = {}
    for key in EVENT_FIELDS:
        if key not in data or data[key] is None:
            continue
        text = _clean_text(data[key])
        if text is None:
            raise ValueError(f"{key} は {MAX_FIELD_LENGTH} 文字以内の1行の文字列にしてください")
        event[key] = text
    if "event" not in event:
        raise ValueError("event（通知の種類）が必要です")
    if data.get("files") is not None:
        event["files"] = parse_files(data["files"])
    if event["event"] in LOCK_EVENTS and not event.get("files"):
        raise ValueError(f"{event['event']} には files（ロック・解除したファイル）が必要です")
    return event


def parse_files(value):
    """files: [{"path", "owner"?, "locked_at"?}]（ロック・解除したファイル）"""
    if not isinstance(value, list) or len(value) > MAX_FILES:
        raise ValueError(f"files は {MAX_FILES} 件までのリストにしてください")
    files = []
    for item in value:
        if not isinstance(item, dict):
            raise ValueError("files の中身はオブジェクトにしてください")
        path = _clean_text(item.get("path"), MAX_PATH_LENGTH)
        if path is None:
            raise ValueError(f"files の path は {MAX_PATH_LENGTH} 文字以内の1行の文字列にしてください")
        entry = {"path": path}
        for key in ("owner", "locked_at"):
            if item.get(key) is not None:
                text = _clean_text(item[key])
                if text is None:
                    raise ValueError(f"files の {key} は {MAX_FIELD_LENGTH} 文字以内の1行の文字列にしてください")
                entry[key] = text
        files.append(entry)
    return files


def _error(status, message):
    return JSONResponse({"ok": False, "error": message}, status_code=status)


def create_app(settings=None, notifier=None):
    settings = settings or Settings.from_env()
    store = SettingsStore(Path(settings.data_dir) / "admin.json")
    notifier = notifier or Notifier(store)
    events = deque(maxlen=settings.max_events)
    ids = itertools.count(1)

    def authorized(request):
        header = request.headers.get("authorization", "")
        scheme, _, token = header.partition(" ")
        return scheme.lower() == "bearer" and same_token(token.strip(), settings.token)

    def require_auth(handler):
        async def wrapped(request):
            if not authorized(request):
                return _error(401, "トークンが正しくありません")
            return await handler(request)
        return wrapped

    async def health(request):
        return JSONResponse({"ok": True})

    @require_auth
    async def ping(request):
        return JSONResponse({"ok": True, "version": __version__})

    @require_auth
    async def post_event(request):
        try:
            body = await _read_body(request)
        except BodyTooLarge:
            return _error(413, f"本文が大きすぎます（上限 {MAX_BODY_BYTES} バイト）")
        try:
            event = parse_event(json.loads(body))
        except json.JSONDecodeError:
            return _error(400, "本文が JSON ではありません")
        except ValueError as e:
            return _error(400, str(e))
        event["id"] = next(ids)
        event["received_at"] = datetime.now(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z")
        events.append(event)
        log.info("受信 #%d event=%s repo=%s remote=%s branch=%s commit=%s user=%s files=%d", event["id"],
                 event["event"], event.get("repo", "-"), event.get("remote", "-"), event.get("branch", "-"),
                 event.get("commit", "-")[:12], event.get("user", "-"), len(event.get("files", [])))
        text = lock_message(event) if notifier.is_configured() else None
        if "files" in event:
            event["file_count"] = len(event["files"])
            event["files"] = event["files"][:STORED_FILES]
        # Post after responding: the CLI gives up after 3 seconds, Discord may take longer
        background = BackgroundTask(post_to_discord, event["id"], text) if text else None
        return JSONResponse({"ok": True, "id": event["id"]}, status_code=201, background=background)

    async def post_to_discord(event_id, text):
        ok, message = await notifier.send(text)
        if not ok:
            log.warning("通知 #%d を Discord に送れませんでした: %s", event_id, message)

    @require_auth
    async def list_events(request):
        return JSONResponse({"ok": True, "events": list(reversed(events))})

    admin, guard = admin_routes(store, notifier, lambda: list(reversed(events)))
    app = Starlette(
        routes=[
            Route("/health", health),
            Route("/api/ping", ping),
            Route("/api/events", post_event, methods=["POST"]),
            Route("/api/events", list_events, methods=["GET"]),
            *admin,
        ],
        middleware=[
            Middleware(TrustedHostMiddleware, allowed_hosts=list(settings.allowed_hosts)),
            Middleware(SecurityHeaders),
        ],
    )
    app.state.store = store
    app.state.guard = guard
    return app
