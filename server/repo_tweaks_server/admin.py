"""管理画面（ログイン、Discord の通知先、パスワードの変更、直近の受信）

作りは cloud-drive-sync の管理画面と同じ。安全のための決まり:
- ログイン必須（パスワードは scrypt のハッシュで確認）。window 秒以内に max_failures 回まちがえたら lock 秒ロックし、通知する
- ログインの状態は HttpOnly・SameSite=Strict の Cookie。POST には CSRF 対策の合言葉が必要で、Origin も確かめる
- 画面の外部読み込み・埋め込みを禁止する（SecurityHeaders）
"""

import asyncio
import os
import re
import secrets
from pathlib import Path
from urllib.parse import urlsplit

from starlette.background import BackgroundTask
from starlette.responses import PlainTextResponse, RedirectResponse
from starlette.routing import Mount, Route
from starlette.staticfiles import StaticFiles
from starlette.templating import Jinja2Templates

from . import __version__
from .lfs_api import parse_repo
from .messages import jst
from .notify import is_valid_webhook, mask_webhook
from .security import LoginGuard, Sessions, hash_password, same_token, verify_password
from .settings import DEFAULT_PASSWORD, MAX_INTERVAL, MAX_STALE_DAYS, MIN_INTERVAL, MIN_STALE_DAYS

HERE = Path(__file__).parent
SESSION_COOKIE = "rt_session"
LOGIN_CSRF_COOKIE = "rt_login_csrf"
MIN_PASSWORD_LENGTH = 8
# Admin forms are a few short fields; anything bigger is refused before parsing (also before login)
MAX_FORM_BYTES = 8 * 1024


class SecurityHeaders:
    """すべての応答に、安全のためのヘッダーを付ける"""

    HEADERS = [
        (b"content-security-policy",
         b"default-src 'self'; img-src 'self'; style-src 'self'; script-src 'self'; "
         b"frame-ancestors 'none'; form-action 'self'; base-uri 'none'"),
        (b"x-content-type-options", b"nosniff"),
        (b"x-frame-options", b"DENY"),
        # Not no-referrer: browsers would then send `Origin: null` on form posts and fail the Origin check
        (b"referrer-policy", b"same-origin"),
        (b"cache-control", b"no-store"),
    ]

    def __init__(self, app):
        self.app = app

    async def __call__(self, scope, receive, send):
        if scope["type"] != "http":
            return await self.app(scope, receive, send)

        async def send_with_headers(message):
            if message["type"] == "http.response.start":
                names = {name for name, _ in self.HEADERS}
                message["headers"] = [h for h in message.get("headers", []) if h[0] not in names] + self.HEADERS
            await send(message)

        await self.app(scope, receive, send_with_headers)


GITHUB_USER = re.compile(r"^[A-Za-z0-9-]{1,39}$")
HHMM = re.compile(r"^([01][0-9]|2[0-3]):[0-5][0-9]$")


def mask_token(token):
    """画面に出すときは、最後の 4 文字だけ見せる"""
    return "…" + token[-4:] if token else ""


def parse_int(text, low, high, what):
    try:
        value = int(str(text).strip())
    except ValueError:
        value = None
    if value is None or not low <= value <= high:
        raise ValueError(f"{what}は {low}〜{high} の整数にしてください")
    return value


def admin_routes(store, notifier, recent_events, reconciler):
    """管理画面のルート。recent_events() は直近の受信（新しい順）を返す"""
    settings = store.load()
    sessions = Sessions(lifetime_seconds=int(settings["session_hours"]) * 3600)
    guard = LoginGuard(**settings["lockout"])

    templates = Jinja2Templates(directory=str(HERE / "templates"))
    templates.env.filters["jst"] = jst

    def current_session(request):
        token = request.cookies.get(SESSION_COOKIE)
        return token, sessions.get(token)

    def same_origin(request):
        # Compare host:port only. Behind an HTTPS reverse proxy the browser says https:// while
        # the server sees http://; the scheme adds nothing against cross-site posts.
        origin = request.headers.get("origin")
        return origin is None or urlsplit(origin).netloc == request.headers.get("host", "")

    def form_too_large(request):
        # Browsers always send Content-Length for these forms; uvicorn refuses bodies longer than declared
        declared = request.headers.get("content-length", "")
        return not declared.isdigit() or int(declared) > MAX_FORM_BYTES

    def too_large():
        return PlainTextResponse("送信された内容が大きすぎます", status_code=413)

    async def password_ok(password):
        # scrypt takes tens of milliseconds; keep the event loop (and the CLI API) responsive
        return await asyncio.to_thread(verify_password, password, store.load()["password"])

    def flash(session, kind, text):
        session["flash"].append({"kind": kind, "text": text})

    def login_required(handler):
        """ログインしていなければログイン画面へ。POST は CSRF の合言葉と Origin を確かめる"""
        async def wrapper(request):
            token, session = current_session(request)
            if session is None:
                if request.method == "GET":
                    return RedirectResponse("/login", status_code=303)
                return PlainTextResponse("ログインしてください", status_code=401)
            form = None
            if request.method == "POST":
                if form_too_large(request):
                    return too_large()
                form = await request.form()
                if not same_origin(request) or not same_token(form.get("csrf"), session["csrf"]):
                    return PlainTextResponse("不正なリクエストです（画面を開き直してください）", status_code=403)
            return await handler(request, token, session, form)
        return wrapper

    def back():
        return RedirectResponse("/", status_code=303)

    def notify_lock_later(reason):
        """Send the lock notice after the response, so a slow Discord does not delay the page"""
        return BackgroundTask(notify_lock, reason)

    async def notify_lock(reason):
        lock = store.load()["lockout"]
        await notifier.send(
            f"🔒 repo-tweaks: {reason}に {lock['max_failures']} 回失敗したため、"
            f"{lock['lock_seconds'] // 60} 分間ログインを止めました。心当たりがなければ、パスワードを変更してください。"
        )

    # --- login ---

    async def login_page(request):
        _, session = current_session(request)
        if session:
            return back()
        # Reuse the token already in the cookie so two tabs or the back button still match
        csrf = request.cookies.get(LOGIN_CSRF_COOKIE, "")
        if not re.fullmatch(r"[0-9a-f]{48}", csrf):
            csrf = os.urandom(24).hex()
        response = templates.TemplateResponse(request, "login.html", {
            "csrf": csrf, "error": request.query_params.get("error"),
            "locked": guard.locked_seconds_left(), "version": __version__,
        })
        response.set_cookie(LOGIN_CSRF_COOKIE, csrf, httponly=True, samesite="strict", max_age=600)
        return response

    async def login_post(request):
        if form_too_large(request):
            return too_large()
        form = await request.form()
        if not same_origin(request) or not same_token(form.get("csrf"), request.cookies.get(LOGIN_CSRF_COOKIE)):
            return RedirectResponse("/login?error=expired", status_code=303)
        if guard.locked_seconds_left():
            return RedirectResponse("/login?error=locked", status_code=303)
        if not await password_ok(form.get("password", "")):
            if guard.record_failure():
                return RedirectResponse("/login?error=locked", status_code=303,
                                        background=notify_lock_later("管理画面のログイン"))
            await asyncio.sleep(1)  # slow down guessing
            return RedirectResponse("/login?error=password", status_code=303)
        guard.record_success()
        token = sessions.create()
        response = back()
        response.set_cookie(SESSION_COOKIE, token, httponly=True, samesite="strict", max_age=sessions.lifetime)
        response.delete_cookie(LOGIN_CSRF_COOKIE)
        return response

    @login_required
    async def logout(request, token, session, form):
        sessions.delete(token)
        response = RedirectResponse("/login", status_code=303)
        response.delete_cookie(SESSION_COOKIE)
        return response

    # --- pages ---

    @login_required
    async def index(request, token, session, form):
        current = store.load()
        messages, session["flash"] = session["flash"], []
        checked = reconciler.snapshot()
        repos = [{
            "id": repo["id"], "remote": repo["remote"], "username": repo["username"],
            "token": mask_token(repo["token"]), "state": checked.get(repo["remote"], {}),
        } for repo in current["repos"]]
        next_run = reconciler.next_run()
        return templates.TemplateResponse(request, "index.html", {
            "csrf": session["csrf"], "flash": messages, "version": __version__,
            "webhook_masked": mask_webhook(current["webhook_url"]),
            "default_password": current["password_is_default"],
            "min_password": MIN_PASSWORD_LENGTH, "events": recent_events()[:20],
            "repos": repos, "reconcile": current["reconcile"], "stale": current["stale"],
            "next_run": next_run.isoformat() if next_run else None,
            "min_interval": MIN_INTERVAL, "max_interval": MAX_INTERVAL,
            "min_stale_days": MIN_STALE_DAYS, "max_stale_days": MAX_STALE_DAYS,
        })

    # --- periodic check ---

    @login_required
    async def add_repo(request, token, session, form):
        username = (form.get("username") or "").strip()
        secret = (form.get("token") or "").strip()
        try:
            remote = parse_repo(form.get("url"))
            if not GITHUB_USER.match(username):
                raise ValueError("GitHub のユーザー名を入れてください（英数字とハイフン、39 文字まで）")
            if not secret or len(secret) > 255 or any(c.isspace() for c in secret):
                raise ValueError("トークンを入れてください（空白なし、255 文字まで）")
            if any(r["remote"] == remote for r in store.load()["repos"]):
                raise ValueError(f"{remote} はもう登録されています（変えるときは、削除してから登録し直してください）")
        except ValueError as e:
            flash(session, "error", f"リポジトリを登録できませんでした: {e}")
            return back()
        repo = {"id": secrets.token_hex(8), "remote": remote, "username": username, "token": secret}
        store.update(lambda s: s["repos"].append(repo))
        reconciler.run_soon()
        flash(session, "ok", f"{remote} を登録しました。すぐに 1 回目の照合をします（1 回目は今のロックを数えるだけで、通知はしません）。")
        return back()

    @login_required
    async def delete_repo(request, token, session, form):
        repo_id = form.get("id", "")
        removed = [r for r in store.load()["repos"] if r["id"] == repo_id]
        if not removed:
            flash(session, "error", "そのリポジトリは登録されていません（画面を開き直してください）。")
            return back()
        store.update(lambda s: s.update({"repos": [r for r in s["repos"] if r["id"] != repo_id]}))
        reconciler.forget(removed[0]["remote"])
        flash(session, "ok", f"{removed[0]['remote']} の登録を削除しました（トークンも消しました）。")
        return back()

    @login_required
    async def save_reconcile(request, token, session, form):
        try:
            minutes = parse_int(form.get("interval_minutes"), MIN_INTERVAL, MAX_INTERVAL, "照合の間隔（分）")
        except ValueError as e:
            flash(session, "error", str(e))
            return back()
        store.update(lambda s: s["reconcile"].update({"interval_minutes": minutes}))
        flash(session, "ok", f"照合の間隔を {minutes} 分にしました。")
        return back()

    @login_required
    async def run_now(request, token, session, form):
        if reconciler.request_run():
            flash(session, "ok", "照合を始めました。少し待ってから画面を開き直すと、結果が出ます。")
        else:
            flash(session, "error", "続けて押されたため、受け付けませんでした。1 分ほど待ってから押してください。")
        return back()

    @login_required
    async def save_stale(request, token, session, form):
        try:
            days = parse_int(form.get("days"), MIN_STALE_DAYS, MAX_STALE_DAYS, "日数")
            time = (form.get("time") or "").strip()
            if not HHMM.match(time):
                raise ValueError("時刻は 00:00〜23:59 の形で入れてください")
        except ValueError as e:
            flash(session, "error", str(e))
            return back()
        enabled = form.get("enabled") == "1"
        store.update(lambda s: s["stale"].update({"enabled": enabled, "days": days, "time": time}))
        flash(session, "ok", f"滞留の警告を{'有効' if enabled else '無効'}にしました（{days} 日以上、毎日 {time}）。" if enabled
              else "滞留の警告を無効にしました。")
        return back()

    @login_required
    async def save_webhook(request, token, session, form):
        url = (form.get("webhook_url") or "").strip()
        if form.get("clear") == "1":
            url = ""
        elif not is_valid_webhook(url):
            flash(session, "error", "Discord の Webhook URL（https://discord.com/api/webhooks/…）を入れてください。")
            return back()
        store.update(lambda s: s.update({"webhook_url": url}))
        flash(session, "ok", "通知先を削除しました。" if not url else "通知先を保存しました。「テスト送信」で確かめられます。")
        return back()

    @login_required
    async def test_webhook(request, token, session, form):
        ok, message = await notifier.send("🔔 repo-tweaks: テスト送信です。この通知が届いていれば設定は正しいです。")
        flash(session, "ok" if ok else "error", f"テスト送信: {message}")
        return back()

    @login_required
    async def change_password(request, token, session, form):
        current, new, confirm = form.get("current", ""), form.get("new", ""), form.get("confirm", "")
        error = None
        if guard.locked_seconds_left():
            error = "ロック中です。しばらくしてから、もう一度試してください。"
        elif not await password_ok(current):
            # Counts toward the login lock, so a logged-in tab cannot be used to guess the password
            if guard.record_failure():
                sessions.delete(token)
                response = RedirectResponse("/login?error=locked", status_code=303,
                                            background=notify_lock_later("パスワードの変更（今のパスワードの確認）"))
                response.delete_cookie(SESSION_COOKIE)
                return response
            await asyncio.sleep(1)
            error = "今のパスワードが違います。"
        elif new == DEFAULT_PASSWORD:
            error = "初期パスワードと同じものは使えません。"
        elif len(new) < MIN_PASSWORD_LENGTH:
            error = f"新しいパスワードは {MIN_PASSWORD_LENGTH} 文字以上にしてください。"
        elif new != confirm:
            error = "新しいパスワード（確認）が一致しません。"
        if error:
            flash(session, "error", f"パスワードを変更できませんでした: {error}")
            return back()
        record = await asyncio.to_thread(hash_password, new)
        store.update(lambda s: s.update({"password": record, "password_is_default": False}))
        sessions.delete_others(token)
        flash(session, "ok", "パスワードを変更しました。ほかのブラウザのログインは切れました。")
        return back()

    routes = [
        Route("/", index),
        Route("/login", login_page, methods=["GET"]),
        Route("/login", login_post, methods=["POST"]),
        Route("/logout", logout, methods=["POST"]),
        Route("/webhook", save_webhook, methods=["POST"]),
        Route("/webhook/test", test_webhook, methods=["POST"]),
        Route("/password", change_password, methods=["POST"]),
        Route("/repos/add", add_repo, methods=["POST"]),
        Route("/repos/delete", delete_repo, methods=["POST"]),
        Route("/reconcile", save_reconcile, methods=["POST"]),
        Route("/reconcile/run", run_now, methods=["POST"]),
        Route("/stale", save_stale, methods=["POST"]),
        Mount("/static", StaticFiles(directory=str(HERE / "static")), name="static"),
    ]
    return routes, guard
