"""ロックの照合・滞留の警告・管理画面の登録のテスト（GitHub と Discord にはつながない）

実行: .venv/bin/python -m unittest discover -s tests -v
"""

import asyncio
import io
import json
import sys
import tempfile
import unittest
import urllib.error
from datetime import datetime, timedelta, timezone
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
sys.path.insert(0, str(Path(__file__).resolve().parent))

from repo_tweaks_server import lfs_api  # noqa: E402
from repo_tweaks_server.lfs_api import LfsApiError, parse_repo  # noqa: E402
from repo_tweaks_server.reconcile import Reconciler, StateStore  # noqa: E402
from repo_tweaks_server.settings import SettingsStore  # noqa: E402
from test_admin import AdminTestCase  # noqa: E402
from test_app import AUTH  # noqa: E402

REMOTE = "github.com/example-org/game"
T0 = datetime(2026, 10, 18, 0, 0, tzinfo=timezone.utc)  # 09:00 in Japan


def lock(lid, path, owner="bob", at="2026-10-17T00:00:00Z"):
    return {"id": lid, "path": path, "owner": owner, "locked_at": at}


class FakeNotifier:
    def __init__(self, configured=True):
        self.sent = []
        self.configured = configured

    def is_configured(self):
        return self.configured

    async def send(self, text):
        self.sent.append(text)
        return True, "送信しました"


class Clock:
    def __init__(self, now):
        self.now = now

    def __call__(self):
        return self.now


class ParseRepoTest(unittest.TestCase):
    def test_accepted_forms(self):
        for url in ("https://github.com/example-org/game", "https://github.com/Example-Org/GAME.git/",
                    "git@github.com:example-org/game.git", "ssh://git@github.com/example-org/game.git"):
            with self.subTest(url=url):
                self.assertEqual(parse_repo(url), REMOTE)

    def test_only_github_repositories(self):
        for url in ("https://gitlab.example.com/a/b", "https://github.com.evil.example/a/b",
                    "https://evil.example/github.com/a/b", "https://github.com/a", "https://github.com/a/b/c",
                    "https://github.com/a/b%2F..", "", "game"):
            with self.subTest(url=url), self.assertRaises(ValueError):
                parse_repo(url)


class FetchLocksTest(unittest.TestCase):
    def respond(self, pages):
        """Fake opener: returns the JSON pages in order and records the requests"""
        self.requests = []

        class Response(io.BytesIO):
            def __enter__(self):
                return self

            def __exit__(self, *args):
                return False

        def fake_open(request, timeout):
            self.requests.append(request)
            page = pages.pop(0)
            if isinstance(page, Exception):
                raise page
            return Response(json.dumps(page).encode())

        return patch.object(lfs_api._OPENER, "open", fake_open)

    def test_pages_and_auth(self):
        pages = [{"locks": [{"id": "1", "path": "a.psd", "owner": {"name": "bob"}, "locked_at": "x"}], "next_cursor": "c2"},
                 {"locks": [{"id": 2, "path": "b.psd", "locked_at": "y"}]}]
        with self.respond(pages):
            locks = lfs_api.fetch_locks(REMOTE, "alice", "secret-token")
        self.assertEqual([l["id"] for l in locks], ["1", "2"])
        self.assertEqual(locks[1]["owner"], "（不明）")
        first, second = self.requests
        self.assertTrue(first.full_url.startswith("https://github.com/example-org/game.git/info/lfs/locks?"))
        self.assertIn("cursor=c2", second.full_url)
        self.assertEqual(first.get_header("Accept"), "application/vnd.git-lfs+json")
        self.assertTrue(first.get_header("Authorization").startswith("Basic "))

    def test_errors_are_explained_without_the_token(self):
        for code, words in ((401, "ログインできません"), (403, "権限"), (404, "見つかりません"), (302, "別の場所")):
            error = urllib.error.HTTPError("u", code, "x", {}, None)
            with self.subTest(code=code), self.respond([error]):
                with self.assertRaises(LfsApiError) as ctx:
                    lfs_api.fetch_locks(REMOTE, "alice", "secret-token")
                self.assertIn(words, str(ctx.exception))
                self.assertNotIn("secret-token", str(ctx.exception))
        with self.respond([urllib.error.URLError("down")]), self.assertRaises(LfsApiError):
            lfs_api.fetch_locks(REMOTE, "alice", "secret-token")

    def test_redirects_are_not_followed(self):
        handler = lfs_api._NoRedirect()
        self.assertIsNone(handler.redirect_request(None, None, 302, "Found", {}, "https://evil.example/"))


class ReconcilerTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.store = SettingsStore(Path(self.tmp.name) / "admin.json")
        self.store.update(lambda s: s["repos"].append({"id": "r1", "remote": REMOTE, "username": "u", "token": "t"}))
        self.locks = []
        self.error = None
        self.notifier = FakeNotifier()
        self.clock = Clock(T0)
        self.reconciler = Reconciler(self.store, StateStore(Path(self.tmp.name) / "state.json"), self.notifier,
                                     self.fetch, clock=self.clock)

    def fetch(self, remote, username, token):
        if self.error:
            raise LfsApiError(self.error)
        return list(self.locks)

    def run_all(self):
        return asyncio.run(self.reconciler.run_all())

    def test_first_check_only_counts(self):
        self.locks = [lock("1", "a.psd")]
        self.run_all()
        self.assertEqual(self.notifier.sent, [])
        self.assertEqual(len(self.reconciler.snapshot()[REMOTE]["locks"]), 1)

    def test_found_and_released(self):
        self.locks = [lock("1", "a.psd")]
        self.run_all()
        self.locks = [lock("2", "Main.unity"), lock("3", "Main.unity.meta")]
        self.run_all()
        found, released = self.notifier.sent
        self.assertEqual(found, "🔍 ロックを見つけました（example-org/game）\n"
                                "・ `Main.unity`（.meta も、bob さん、10/17 09:00 から）")
        self.assertEqual(released, "🔓 ロックが外れました（example-org/game）\n"
                                   "・ `a.psd`（bob さんのロック、10/17 09:00 から）")
        # Nothing changed: nothing new
        self.run_all()
        self.assertEqual(len(self.notifier.sent), 2)

    def test_cli_notifications_are_not_repeated(self):
        self.run_all()
        self.reconciler.record_cli_event({"event": "lock", "remote": REMOTE, "files": [{"path": "a.psd", "id": "1"}]})
        self.locks = [lock("1", "a.psd")]
        self.run_all()
        self.assertEqual(self.notifier.sent, [])
        self.reconciler.record_cli_event({"event": "unlock", "remote": REMOTE, "files": [{"path": "a.psd", "id": "1"}]})
        self.locks = []
        self.run_all()
        self.assertEqual(self.notifier.sent, [])

    def test_failures_are_told_once_and_recovery_too(self):
        self.run_all()
        self.error = "GitHub にログインできませんでした（401）"
        with self.assertLogs("repo_tweaks", "WARNING"):
            self.run_all()
            self.run_all()
        self.assertEqual(len(self.notifier.sent), 1)
        self.assertIn("照合ができませんでした", self.notifier.sent[0])
        self.assertIn("401", self.reconciler.snapshot()[REMOTE]["last_error"])
        self.error = None
        self.run_all()
        self.assertEqual(self.notifier.sent[1], "✅ ロックの照合が直りました（example-org/game）")
        self.assertIsNone(self.reconciler.snapshot()[REMOTE]["last_error"])

    def test_no_webhook_means_no_sending(self):
        self.notifier.configured = False
        self.run_all()
        self.locks = [lock("1", "a.psd")]
        self.run_all()
        self.assertEqual(self.notifier.sent, [])

    def test_stale_warning_once_a_day_after_the_time(self):
        self.locks = [lock("1", "old.psd", at="2026-10-14T00:00:00Z"), lock("2", "new.psd", at="2026-10-17T12:00:00Z")]
        self.run_all()
        stale = lambda: asyncio.run(self.reconciler.check_stale())  # noqa: E731
        self.assertFalse(stale())  # 09:00 in Japan, before 10:00
        self.clock.now = T0 + timedelta(hours=1)  # 10:00
        self.assertTrue(stale())
        self.assertEqual(self.notifier.sent[-1], "⏰ 3日以上ロックされたままのものが 1 件あります\n"
                                                 "・ `old.psd`（example-org/game、bob さん、10/14 09:00 から、4日1時間）")
        self.clock.now = T0 + timedelta(hours=5)
        self.assertFalse(stale())  # already sent today
        self.clock.now = T0 + timedelta(days=1, hours=1)
        self.assertTrue(stale())  # next day
        self.store.update(lambda s: s["stale"].update({"enabled": False}))
        self.clock.now = T0 + timedelta(days=2, hours=1)
        self.assertFalse(stale())

    def test_cli_remote_case_does_not_matter(self):
        self.run_all()
        self.reconciler.record_cli_event({"event": "lock", "remote": "github.com/Example-Org/Game",
                                          "files": [{"path": "a.psd", "id": "1"}]})
        self.locks = [lock("1", "a.psd")]
        self.run_all()
        self.assertEqual(self.notifier.sent, [])

    def test_unexpected_error_is_recorded_and_others_still_run(self):
        self.store.update(lambda s: s["repos"].insert(0, {"id": "r0", "remote": "github.com/example-org/broken",
                                                          "username": "u", "token": "t"}))

        def fetch(remote, username, token):
            if remote.endswith("broken"):
                return [None]  # not a lock: the conversion fails with an unexpected error
            return list(self.locks)
        self.reconciler.fetcher = fetch
        with self.assertLogs("repo_tweaks", "ERROR"):
            self.run_all()
        snapshot = self.reconciler.snapshot()
        self.assertIn("予期しないエラー", snapshot["github.com/example-org/broken"]["last_error"])
        self.assertTrue(snapshot[REMOTE]["initialized"])

    def test_removed_while_checking_is_not_written_back(self):
        self.run_all()

        def fetch_then_delete(remote, username, token):
            self.store.update(lambda s: s.update({"repos": []}))
            self.reconciler.forget(remote)
            return [lock("1", "a.psd")]
        self.reconciler.fetcher = fetch_then_delete
        self.run_all()
        self.assertNotIn(REMOTE, self.reconciler.snapshot())

    def test_just_taken_locks_wait_one_round_for_the_cli(self):
        self.run_all()
        self.locks = [lock("1", "a.psd", at="2026-10-17T23:59:30Z")]  # 30 seconds before T0
        self.run_all()
        self.assertEqual(self.notifier.sent, [])  # held over
        self.clock.now = T0 + timedelta(minutes=10)
        self.run_all()
        self.assertEqual(len(self.notifier.sent), 1)
        self.assertIn("a.psd", self.notifier.sent[0])

    def test_held_over_lock_is_dropped_when_the_cli_reports_it(self):
        self.run_all()
        self.locks = [lock("1", "a.psd", at="2026-10-17T23:59:30Z")]
        self.run_all()
        self.reconciler.record_cli_event({"event": "lock", "remote": REMOTE, "files": [{"path": "a.psd", "id": "1"}]})
        self.clock.now = T0 + timedelta(minutes=10)
        self.run_all()
        self.assertEqual(self.notifier.sent, [])

    def test_stale_leaves_out_failing_repositories(self):
        self.store.update(lambda s: s["repos"].append({"id": "r2", "remote": "github.com/example-org/other",
                                                       "username": "u", "token": "t"}))
        self.locks = [lock("1", "old.psd", at="2026-10-14T00:00:00Z")]
        self.run_all()
        original = self.reconciler.fetcher
        self.reconciler.fetcher = lambda remote, *a: (_ for _ in ()).throw(LfsApiError("401")) \
            if remote.endswith("other") else original(remote, *a)
        with self.assertLogs("repo_tweaks", "WARNING"):
            self.run_all()
        self.clock.now = T0 + timedelta(hours=1)
        asyncio.run(self.reconciler.check_stale())
        text = self.notifier.sent[-1]
        self.assertIn("1 件あります", text)
        self.assertTrue(text.endswith("※ 照合に失敗しているため含めていません: example-org/other"))

    def test_broken_stale_time_falls_back_to_ten(self):
        self.locks = [lock("1", "old.psd", at="2026-10-14T00:00:00Z")]
        self.run_all()
        self.store.update(lambda s: s["stale"].update({"time": "10"}))
        self.assertFalse(asyncio.run(self.reconciler.check_stale()))  # 09:00
        self.clock.now = T0 + timedelta(hours=1)
        self.assertTrue(asyncio.run(self.reconciler.check_stale()))

    def test_forget_and_manual_rate_limit(self):
        self.run_all()
        self.reconciler.forget(REMOTE)
        self.assertNotIn(REMOTE, self.reconciler.snapshot())
        self.assertTrue(self.reconciler.request_run())
        self.assertFalse(self.reconciler.request_run())
        self.clock.now = T0 + timedelta(seconds=61)
        self.assertTrue(self.reconciler.request_run())


class ApiRecordsCliEventsTest(AdminTestCase):
    def test_lock_ids_from_the_cli_are_remembered(self):
        self.client.post("/api/events", headers=AUTH, json={
            "event": "lock", "remote": REMOTE, "user": "alice", "files": [{"path": "a.psd", "id": "9"}]})
        state = json.loads((Path(self.client.tmp.name) / "state.json").read_text())
        self.assertIn("9", state["notified"][REMOTE]["lock"])


class AdminRepoTest(AdminTestCase):
    def setUp(self):
        super().setUp()
        self.login()

    def repos(self):
        return self.client.app.state.store.load()["repos"]

    def test_register_masks_token_and_delete_forgets(self):
        self.post("/repos/add", {"url": "https://github.com/example-org/game.git", "username": "alice",
                                 "token": "dummy-token-for-tests-1234"})
        [repo] = self.repos()
        self.assertEqual(repo["remote"], REMOTE)
        html = self.page()
        self.assertIn("…1234", html)
        self.assertNotIn("dummy-token-for-tests-1234", html)
        self.post("/repos/delete", {"id": repo["id"]})
        self.assertEqual(self.repos(), [])
        self.assertNotIn("dummy-token-for-tests-1234", (Path(self.client.tmp.name) / "admin.json").read_text())

    def test_register_rejects_bad_input(self):
        good = {"url": "https://github.com/example-org/game", "username": "alice", "token": "tok"}
        cases = [
            ({**good, "url": "https://gitlab.example.com/a/b"}, "GitHub"),
            ({**good, "username": "a b"}, "ユーザー名"),
            ({**good, "token": ""}, "トークン"),
            ({**good, "token": "has space"}, "トークン"),
        ]
        for data, words in cases:
            with self.subTest(words=words):
                self.post("/repos/add", data)
                self.assertEqual(self.repos(), [])
                self.assertIn(words, self.page())
        self.post("/repos/add", good)
        self.post("/repos/add", good)
        self.assertEqual(len(self.repos()), 1)
        self.assertIn("もう登録されています", self.page())

    def test_interval_and_stale_settings(self):
        self.post("/reconcile", {"interval_minutes": "5"})
        self.assertIn("10〜1440", self.page())
        self.post("/reconcile", {"interval_minutes": "30"})
        self.assertEqual(self.client.app.state.store.load()["reconcile"]["interval_minutes"], 30)
        self.post("/stale", {"enabled": "1", "days": "0", "time": "10:00"})
        self.assertIn("日数", self.page())
        self.post("/stale", {"enabled": "1", "days": "5", "time": "25:00"})
        self.assertIn("時刻", self.page())
        self.post("/stale", {"days": "5", "time": "18:30"})
        self.assertEqual(self.client.app.state.store.load()["stale"], {"enabled": False, "days": 5, "time": "18:30"})

    def test_run_now_is_rate_limited(self):
        self.post("/reconcile/run", {})
        self.assertIn("照合を始めました", self.page())
        self.post("/reconcile/run", {})
        self.assertIn("1 分ほど待って", self.page())

    def test_errors_show_on_the_page(self):
        self.post("/repos/add", {"url": "https://github.com/example-org/game", "username": "alice", "token": "tok"})
        reconciler = self.client.app.state.reconciler
        reconciler.fetcher = lambda *a: (_ for _ in ()).throw(LfsApiError("GitHub にログインできませんでした（401）"))
        with self.assertLogs("repo_tweaks", "WARNING"):
            asyncio.run(reconciler.run_all())
        self.assertIn("ログインできませんでした（401）", self.page())


if __name__ == "__main__":
    unittest.main()
