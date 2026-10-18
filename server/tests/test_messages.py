"""ロック・解除の通知（文面と、Discord への送信）のテスト。Discord にはつながない

実行: .venv/bin/python -m unittest discover -s tests -v
"""

import json
import sys
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
sys.path.insert(0, str(Path(__file__).resolve().parent))

from repo_tweaks_server import notify  # noqa: E402
from repo_tweaks_server.messages import MAX_LISTED_FILES, MAX_MESSAGE_CHARS, lock_message  # noqa: E402
from test_app import AUTH, make_client  # noqa: E402

BASE = {"remote": "github.com/example-org/game", "repo": "game", "user": "alice"}


def files(*paths, **extra):
    return [{"path": p, **extra} for p in paths]


class MessageTest(unittest.TestCase):
    def test_lock_folds_meta_into_its_asset(self):
        text = lock_message({**BASE, "event": "lock",
                             "files": files("Assets/Main.unity", "Assets/Main.unity.meta", "Assets/hero.psd")})
        self.assertEqual(text, "🔒 alice がロックしました（example-org/game）\n"
                               "・ `Assets/Main.unity`（.meta も）\n"
                               "・ `Assets/hero.psd`")

    def test_unlock(self):
        text = lock_message({**BASE, "event": "unlock", "files": files("Assets/Main.unity")})
        self.assertEqual(text, "🔓 alice が解除しました（example-org/game）\n・ `Assets/Main.unity`")

    def test_force_unlock_names_the_owner_and_time(self):
        text = lock_message({**BASE, "event": "force_unlock",
                             "files": files("Assets/hero.psd", owner="bob", locked_at="2026-10-14T01:00:00Z")})
        self.assertEqual(text, "⚠️ alice が bob さんのロックを強制解除しました（example-org/game）\n"
                               "・ `Assets/hero.psd`（10/14 10:00 から）")

    def test_force_unlock_of_several_owners(self):
        text = lock_message({**BASE, "event": "force_unlock",
                             "files": [{"path": "a.psd", "owner": "bob"}, {"path": "b.psd", "owner": "carol"}]})
        self.assertTrue(text.startswith("⚠️ alice が他人のロックを強制解除しました"))
        self.assertIn("・ `a.psd`（bob さんのロック）", text)
        self.assertIn("・ `b.psd`（carol さんのロック）", text)

    def test_long_lists_are_cut(self):
        paths = [f"f{i}.psd" for i in range(MAX_LISTED_FILES + 5)]
        text = lock_message({**BASE, "event": "lock", "files": files(*paths)})
        self.assertEqual(text.count("・ "), MAX_LISTED_FILES)
        self.assertTrue(text.endswith("ほか 5 件"))

    def test_long_paths_stay_within_discord_limits(self):
        paths = [f"Assets/Game/Characters/Enemies/{'Boss' * 25}/Prefabs/Variant{i}.prefab" for i in range(20)]
        text = lock_message({**BASE, "event": "lock", "files": files(*paths)})
        self.assertLessEqual(len(text), MAX_MESSAGE_CHARS)
        last = text.splitlines()[-1]
        self.assertRegex(last, r"^ほか \d+ 件$")
        # Every listed path is a complete code span, cut from the front so the file name stays
        for line in text.splitlines()[1:-1]:
            self.assertEqual(line.count("`"), 2)
            self.assertIn("Variant", line)
        self.assertIn("`…", text)

    def test_meta_with_another_owner_is_listed_separately(self):
        text = lock_message({**BASE, "event": "force_unlock", "files": [
            {"path": "Main.unity", "owner": "bob"}, {"path": "Main.unity.meta", "owner": "carol"}]})
        self.assertIn("・ `Main.unity`（bob さんのロック）", text)
        self.assertIn("・ `Main.unity.meta`（carol さんのロック）", text)
        self.assertNotIn(".meta も", text)

    def test_user_text_cannot_format_or_break_code(self):
        text = lock_message({**BASE, "user": "*bold*", "event": "lock", "files": files("a`b.psd")})
        self.assertIn("\\*bold\\*", text)
        self.assertIn("`a'b.psd`", text)

    def test_repo_label_falls_back_to_folder_name(self):
        text = lock_message({"repo": "game", "user": "alice", "event": "lock", "files": files("a.psd")})
        self.assertIn("（game）", text)

    def test_other_events_have_no_message(self):
        self.assertIsNone(lock_message({**BASE, "event": "post-commit"}))
        self.assertIsNone(lock_message({**BASE, "event": "lock"}))


class FakeNotifier:
    def __init__(self, configured=True):
        self.sent = []
        self.configured = configured

    def is_configured(self):
        return self.configured

    async def send(self, text):
        self.sent.append(text)
        return True, "送信しました"


class LockEventApiTest(unittest.TestCase):
    def setUp(self):
        self.notifier = FakeNotifier()
        self.client = make_client(notifier=self.notifier)

    def post(self, payload):
        return self.client.post("/api/events", json=payload, headers=AUTH)

    def test_lock_event_is_posted_to_discord(self):
        r = self.post({**BASE, "event": "lock", "files": files("Assets/Main.unity", "Assets/Main.unity.meta")})
        self.assertEqual(r.status_code, 201)
        self.assertEqual(self.notifier.sent, ["🔒 alice がロックしました（example-org/game）\n"
                                              "・ `Assets/Main.unity`（.meta も）"])
        listed = self.client.get("/api/events", headers=AUTH).json()["events"][0]
        self.assertEqual([f["path"] for f in listed["files"]], ["Assets/Main.unity", "Assets/Main.unity.meta"])

    def test_nothing_is_tried_without_a_webhook(self):
        self.notifier.configured = False
        with self.assertNoLogs("repo_tweaks", "WARNING"):
            r = self.post({**BASE, "event": "lock", "files": files("a.psd")})
        self.assertEqual(r.status_code, 201)
        self.assertEqual(self.notifier.sent, [])

    def test_only_a_few_paths_are_kept_in_memory(self):
        paths = [f"f{i}.psd" for i in range(50)]
        self.post({**BASE, "event": "lock", "files": files(*paths)})
        stored = self.client.get("/api/events", headers=AUTH).json()["events"][0]
        self.assertEqual(stored["file_count"], 50)
        self.assertEqual(len(stored["files"]), 20)
        # The Discord message was built from all of them
        self.assertIn("ほか", self.notifier.sent[0])

    def test_other_events_are_not_posted(self):
        self.post({**BASE, "event": "post-commit"})
        self.assertEqual(self.notifier.sent, [])

    def test_lock_event_needs_files(self):
        r = self.post({**BASE, "event": "lock"})
        self.assertEqual(r.status_code, 400)
        self.assertIn("files", r.json()["error"])

    def test_files_are_validated(self):
        bad = [
            "not a list",
            [{"path": ""}],
            [{"path": "a\nb"}],
            [{"path": "a" * 1025}],
            ["just a string"],
            [{"path": "a", "owner": 3}],
            [{"path": f"f{i}"} for i in range(201)],
        ]
        for value in bad:
            with self.subTest(value=str(value)[:40]):
                r = self.post({**BASE, "event": "lock", "files": value})
                self.assertEqual(r.status_code, 400)
        self.assertEqual(self.notifier.sent, [])

    def test_two_hundred_long_paths_fit_in_the_body_limit(self):
        paths = [f"Assets/{'x' * 200}/{i}.psd" for i in range(200)]
        r = self.post({**BASE, "event": "lock", "files": files(*paths)})
        self.assertEqual(r.status_code, 201)

    def test_discord_failure_does_not_fail_the_request(self):
        async def fail(text):
            return False, "送信に失敗しました（Discord の応答 500）"
        self.notifier.send = fail
        with self.assertLogs("repo_tweaks", "WARNING"):
            r = self.post({**BASE, "event": "unlock", "files": files("a.psd")})
        self.assertEqual(r.status_code, 201)


class DiscordPayloadTest(unittest.TestCase):
    def test_mentions_are_disabled(self):
        sent = {}

        class Response:
            status = 204

            def __enter__(self):
                return self

            def __exit__(self, *args):
                return False

        def fake_urlopen(request, timeout):
            sent["body"] = json.loads(request.data)
            return Response()

        with patch.object(notify.urllib.request, "urlopen", fake_urlopen):
            ok, _ = notify._post("https://discord.com/api/webhooks/1/x", "@everyone a.psd")
        self.assertTrue(ok)
        self.assertEqual(sent["body"]["allowed_mentions"], {"parse": []})
        self.assertEqual(sent["body"]["content"], "@everyone a.psd")


if __name__ == "__main__":
    unittest.main()
