"""ロックの定期的な照合と、滞留の警告

- 登録したリポジトリのロック一覧を一定の間隔で GitHub から取り、前回と比べる
  - 増えたロックで、まだ通知していないもの → 「見つけました」（CLI を通らずに取られたもの）
  - 消えたロックで、まだ通知していないもの → 「外れました」
  - 初回は数えるだけで通知しない（登録した瞬間に、今あるロックがまとめて流れないように）
- 二重に通知しないよう、「リポジトリ + ロックの ID」ごとに通知済みかを state.json に残す
  （CLI からの lock / unlock / force_unlock のイベントも、ここに記録する）
- 照合に失敗し始めたとき・直ったときは、1 回ずつ通知する（毎回は送らない）
- 毎日決まった時刻（日本時間）に、N 日以上ロックされたままのものをまとめて通知する
  （照合で保存した状態を使い、GitHub への追加の問い合わせはしない）
"""

import asyncio
import contextlib
import copy
import json
import logging
import os
import threading
from datetime import datetime, timedelta, timezone
from pathlib import Path

from . import messages
from .lfs_api import LfsApiError

log = logging.getLogger("repo_tweaks")

JST = timezone(timedelta(hours=9))
# How long to remember "already notified" for a lock ID that the check never saw
# (e.g. locked and unlocked with the CLI between two checks)
NOTIFIED_KEEP = timedelta(days=7)
MANUAL_MIN_INTERVAL = timedelta(seconds=60)
# A lock taken this recently may still be on its way from the CLI (which notifies it with its ID);
# wait one more round before calling it "found" so it is not announced twice
FRESH_LOCK_GRACE = timedelta(minutes=2)
DEFAULT_STALE_TIME = (10, 0)
TICK_SECONDS = 30


def _now():
    return datetime.now(timezone.utc)


def _iso(dt):
    return dt.astimezone(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z")


def _parse(value):
    try:
        return datetime.fromisoformat(value.replace("Z", "+00:00"))
    except (AttributeError, ValueError):
        return None


class StateStore:
    """state.json: 前回のロック一覧・通知済みの ID・前回の照合の結果・滞留の警告を送った日

    {"repos": {remote: {"initialized", "locks": {id: lock}, "last_checked", "last_error"}},
     "notified": {remote: {"lock": {id: iso}, "unlock": {id: iso}}},
     "stale_sent_on": "YYYY-MM-DD"}
    """

    def __init__(self, path):
        self.path = Path(path)
        self._lock = threading.Lock()

    def load(self):
        with self._lock:
            try:
                data = json.loads(self.path.read_text(encoding="utf-8"))
            except FileNotFoundError:
                data = {}
            except (OSError, ValueError):
                log.warning("state.json を読めなかったため、照合の状態を最初からやり直します")
                data = {}
        data.setdefault("repos", {})
        data.setdefault("notified", {})
        data.setdefault("stale_sent_on", None)
        return data

    def save(self, data):
        with self._lock:
            self.path.parent.mkdir(parents=True, exist_ok=True)
            tmp = self.path.with_name(f".{self.path.name}.tmp")
            tmp.write_text(json.dumps(data, ensure_ascii=False, indent=1), encoding="utf-8")
            os.replace(tmp, self.path)


def _notified(data, remote):
    entry = data["notified"].setdefault(remote, {})
    entry.setdefault("lock", {})
    entry.setdefault("unlock", {})
    return entry


def diff(previous, current, notified, pending=()):
    """前回と今回のロック（{id: lock}）を比べ、通知するものを返す: (見つけた, 外れた)

    pending: 前回、取られて間もないため通知を持ち越したロックの ID（今回あらためて判断する）
    """
    found = [lock for lid, lock in current.items()
             if (lid not in previous or lid in pending) and lid not in notified["lock"]]
    released = [lock for lid, lock in previous.items() if lid not in current and lid not in notified["unlock"]]
    return found, released


def _is_fresh(lock, now):
    """Taken within FRESH_LOCK_GRACE (an unreadable time counts as old)"""
    locked_at = _parse(lock.get("locked_at", ""))
    return locked_at is not None and now - locked_at < FRESH_LOCK_GRACE


def _stale_time(text):
    """"HH:MM" -> (h, m). A broken value (hand-edited admin.json) falls back to 10:00"""
    try:
        hour, minute = (int(x) for x in str(text).split(":"))
        if 0 <= hour < 24 and 0 <= minute < 60:
            return hour, minute
    except ValueError:
        pass
    return DEFAULT_STALE_TIME


class Reconciler:
    def __init__(self, store, state, notifier, fetcher, clock=_now):
        """store: 管理画面の設定（repos / reconcile / stale）。fetcher(remote, username, token) -> [lock]"""
        self.store = store
        self.state = state
        self.notifier = notifier
        self.fetcher = fetcher
        self.clock = clock
        self._running = asyncio.Lock()
        self._wake = None
        self.last_run = None
        self.last_manual = None
        # Set when a round held fresh locks over: check again once they are old enough,
        # instead of waiting for the regular interval
        self._recheck_at = None

    # --- CLI events ---

    def record_cli_event(self, event):
        """CLI がすでに通知したロック・解除を覚えておく（照合で同じものを流さないため）"""
        kind, remote = event.get("event"), (event.get("remote") or "").lower()
        if kind not in messages.LOCK_EVENTS or not remote:
            return
        ids = [f["id"] for f in event.get("files", []) if f.get("id")]
        if not ids:
            return
        data = self.state.load()
        bucket = _notified(data, remote)["lock" if kind == "lock" else "unlock"]
        now = _iso(self.clock())
        for lid in ids:
            bucket[lid] = now
        self.state.save(data)

    # --- checking ---

    async def run_all(self):
        """登録したすべてのリポジトリを照合する。戻り値は利用者向けの結果の説明"""
        async with self._running:
            repos = self.store.load()["repos"]
            self.last_run = self.clock()
            self._recheck_at = None
            results = []
            for repo in repos:
                results.append(await self._run_repo(repo))
            return results

    def _registered(self, repo):
        """Still registered (not deleted, nor deleted and added again) while we were fetching"""
        return any(r["id"] == repo["id"] for r in self.store.load()["repos"])

    async def _run_repo(self, repo):
        remote = repo["remote"]
        try:
            fetched = await asyncio.to_thread(self.fetcher, remote, repo["username"], repo["token"])
            current = {lock["id"]: lock for lock in fetched if lock.get("id")}
        except LfsApiError as e:
            return await self._record_error(repo, str(e))
        except Exception as e:  # an odd response must not stop the other repositories
            log.exception("照合の途中で予期しないエラーが起きました（%s）", remote)
            return await self._record_error(repo, f"照合の途中で予期しないエラーが起きました（{type(e).__name__}）")
        if not self._registered(repo):
            return f"{remote}: 照合の途中で登録が削除されたため、結果を捨てました"
        now = self.clock()

        data = self.state.load()
        entry = data["repos"].setdefault(remote, {"initialized": False, "locks": {}, "last_error": None})
        notified = _notified(data, remote)
        texts = []
        pending = []
        if entry.get("last_error"):
            texts.append(messages.check_recovered_message(remote))
        if entry.get("initialized"):
            found, released = diff(entry.get("locks", {}), current, notified, entry.get("pending", []))
            pending = [lock["id"] for lock in found if _is_fresh(lock, now)]
            found = [lock for lock in found if lock["id"] not in pending]
            if found:
                texts.append(messages.found_message(remote, found))
            if released:
                texts.append(messages.released_message(remote, released))
        # The current locks count as notified from now on (except the ones held over);
        # forget what is gone or too old
        for lid in current:
            if lid not in pending:
                notified["lock"].setdefault(lid, _iso(now))
        for lid in [lid for lid in notified["unlock"] if lid in entry.get("locks", {}) and lid not in current]:
            del notified["unlock"][lid]
        for bucket in (notified["lock"], notified["unlock"]):
            for lid in list(bucket):
                seen = _parse(bucket[lid])
                if lid not in current and (seen is None or now - seen > NOTIFIED_KEEP):
                    del bucket[lid]
        if pending:
            self._recheck_at = now + FRESH_LOCK_GRACE
        entry.update({"initialized": True, "locks": current, "pending": pending,
                      "last_checked": _iso(now), "last_error": None})
        self.state.save(data)
        for text in texts:
            await self._send(text)
        return f"{remote}: ロック {len(current)} 件"

    async def _record_error(self, repo, reason):
        remote = repo["remote"]
        if not self._registered(repo):
            return f"{remote}: 照合の途中で登録が削除されました"
        data = self.state.load()
        entry = data["repos"].setdefault(remote, {"initialized": False, "locks": {}, "last_error": None})
        first_failure = not entry.get("last_error")
        entry.update({"last_checked": _iso(self.clock()), "last_error": reason})
        self.state.save(data)
        log.warning("照合に失敗しました（%s）: %s", remote, reason)
        if first_failure:
            await self._send(messages.check_failed_message(remote, reason))
        return f"{remote}: {reason}"

    async def _send(self, text):
        if not self.notifier.is_configured():
            return
        ok, message = await self.notifier.send(text)
        if not ok:
            log.warning("照合の通知を Discord に送れませんでした: %s", message)

    def forget(self, remote):
        """リポジトリの登録を消したとき、その状態も消す"""
        data = self.state.load()
        data["repos"].pop(remote, None)
        data["notified"].pop(remote, None)
        self.state.save(data)

    # --- stale warning ---

    async def check_stale(self):
        """毎日決まった時刻に 1 回、N 日以上ロックされたままのものを通知する。送ったら True"""
        stale = self.store.load()["stale"]
        if not stale.get("enabled"):
            return False
        now = self.clock()
        local = now.astimezone(JST)
        hour, minute = _stale_time(stale.get("time"))
        today = local.date().isoformat()
        data = self.state.load()
        if data.get("stale_sent_on") == today or (local.hour, local.minute) < (hour, minute):
            return False
        days = int(stale["days"])
        registered = {repo["remote"] for repo in self.store.load()["repos"]}
        items = []
        failing = []
        for remote, entry in data["repos"].items():
            if remote not in registered:
                continue
            # A failing check means the saved list may be out of date: leave it out and say so
            if entry.get("last_error"):
                failing.append(remote)
                continue
            for lock in entry.get("locks", {}).values():
                locked_at = _parse(lock.get("locked_at", ""))
                if locked_at and now - locked_at >= timedelta(days=days):
                    items.append((remote, lock))
        items.sort(key=lambda item: item[1].get("locked_at", ""))
        # Mark the day as done even when there is nothing to report
        data["stale_sent_on"] = today
        self.state.save(data)
        if items:
            await self._send(messages.stale_message(items, days, now, failing))
        return bool(items)

    # --- schedule ---

    def next_run(self):
        if self.last_run is None:
            return None
        minutes = int(self.store.load()["reconcile"]["interval_minutes"])
        regular = self.last_run + timedelta(minutes=minutes)
        return min(regular, self._recheck_at) if self._recheck_at else regular

    def run_soon(self):
        """Check on the next tick (after a repository was added); not rate-limited"""
        self.last_run = None
        if self._wake:
            self._wake.set()

    def request_run(self):
        """管理画面の「今すぐ照合」。連打は MANUAL_MIN_INTERVAL に 1 回まで。受け付けたら True"""
        now = self.clock()
        if self.last_manual and now - self.last_manual < MANUAL_MIN_INTERVAL:
            return False
        self.last_manual = now
        self.last_run = None  # due now
        if self._wake:
            self._wake.set()
        return True

    async def loop(self):
        self._wake = asyncio.Event()
        while True:
            due = self.next_run()
            try:
                if due is None or self.clock() >= due:
                    await self.run_all()
                await self.check_stale()
            except Exception:  # keep the schedule alive; one bad round must not stop it
                log.exception("照合の途中で予期しないエラーが起きました")
            with contextlib.suppress(asyncio.TimeoutError):
                await asyncio.wait_for(self._wake.wait(), timeout=TICK_SECONDS)
            self._wake.clear()

    def snapshot(self):
        """管理画面用: リポジトリごとの前回の照合の結果"""
        return copy.deepcopy(self.state.load()["repos"])
