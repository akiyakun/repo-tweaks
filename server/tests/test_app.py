"""サーバーのテスト（外部通信なし）

実行: .venv/bin/python -m unittest discover -s tests -v
"""

import atexit
import sys
import tempfile
import unittest
from pathlib import Path

from starlette.testclient import TestClient

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from repo_tweaks_server.app import MAX_BODY_BYTES, create_app  # noqa: E402
from repo_tweaks_server.config import Settings  # noqa: E402

TOKEN = "t" * 40
AUTH = {"Authorization": f"Bearer {TOKEN}"}


# Data folders for test clients; kept referenced and removed when the test run ends
_TEMP_DIRS = []
atexit.register(lambda: [t.cleanup() for t in _TEMP_DIRS])


def make_client(max_events=100, notifier=None, data_dir=None, fetcher=None):
    """A client with its own data folder, or with `data_dir` to share one (like a restarted server)."""
    tmp = None
    if not data_dir:
        tmp = tempfile.TemporaryDirectory()
        _TEMP_DIRS.append(tmp)
    settings = Settings(token=TOKEN, allowed_hosts=("testserver",), max_events=max_events,
                        data_dir=data_dir or tmp.name)
    app = create_app(settings, notifier=notifier, fetcher=fetcher or (lambda *a: []), run_scheduler=False)
    client = TestClient(app, follow_redirects=False)
    client.tmp = tmp
    return client


class SettingsTest(unittest.TestCase):
    def test_token_required(self):
        with self.assertRaises(ValueError):
            Settings.from_env({})
        with self.assertRaises(ValueError):
            Settings.from_env({"REPO_TWEAKS_TOKEN": "short"})

    def test_wildcard_host_rejected(self):
        with self.assertRaises(ValueError):
            Settings.from_env({"REPO_TWEAKS_TOKEN": TOKEN, "REPO_TWEAKS_ALLOWED_HOSTS": "*"})

    def test_max_events_must_be_integer(self):
        for value in ("100件", "", "0", "1001"):
            with self.subTest(value=value), self.assertRaisesRegex(ValueError, "REPO_TWEAKS_MAX_EVENTS"):
                Settings.from_env({"REPO_TWEAKS_TOKEN": TOKEN, "REPO_TWEAKS_MAX_EVENTS": value})

    def test_defaults(self):
        s = Settings.from_env({"REPO_TWEAKS_TOKEN": TOKEN})
        self.assertEqual(s.allowed_hosts, ("localhost", "127.0.0.1"))
        self.assertEqual(s.max_events, 100)
        # Always server/data/dev (git-ignored), not relative to where the server was started
        self.assertEqual(Path(s.data_dir), Path(__file__).resolve().parent.parent / "data" / "dev")
        self.assertEqual(Settings.from_env({"REPO_TWEAKS_TOKEN": TOKEN, "REPO_TWEAKS_DATA_DIR": "/data"}).data_dir, "/data")


class AppTest(unittest.TestCase):
    def setUp(self):
        self.client = make_client()

    def test_health_without_auth(self):
        self.assertEqual(self.client.get("/health").status_code, 200)

    def test_api_requires_token(self):
        self.assertEqual(self.client.get("/api/ping").status_code, 401)
        self.assertEqual(self.client.get("/api/ping", headers={"Authorization": "Bearer wrong"}).status_code, 401)
        self.assertEqual(self.client.post("/api/events", json={"event": "x"}).status_code, 401)
        self.assertEqual(self.client.get("/api/events").status_code, 401)

    def test_ping(self):
        r = self.client.get("/api/ping", headers=AUTH)
        self.assertEqual(r.status_code, 200)
        self.assertTrue(r.json()["ok"])

    def test_unknown_host_rejected(self):
        r = self.client.get("/health", headers={"Host": "evil.example"})
        self.assertEqual(r.status_code, 400)

    def test_post_and_list(self):
        payload = {"event": "post-commit", "repo": "demo", "branch": "main", "commit": "abc123",
                   "user": "alice", "extra": "dropped"}
        r = self.client.post("/api/events", json=payload, headers=AUTH)
        self.assertEqual(r.status_code, 201)
        self.assertEqual(r.json()["id"], 1)
        events = self.client.get("/api/events", headers=AUTH).json()["events"]
        self.assertEqual(len(events), 1)
        self.assertEqual(events[0]["repo"], "demo")
        self.assertNotIn("extra", events[0])
        self.assertTrue(events[0]["received_at"].endswith("Z"))

    def test_list_is_newest_first_and_capped(self):
        client = make_client(max_events=2)
        for i in range(3):
            client.post("/api/events", json={"event": f"e{i}"}, headers=AUTH)
        events = client.get("/api/events", headers=AUTH).json()["events"]
        self.assertEqual([e["event"] for e in events], ["e2", "e1"])

    def test_invalid_payloads(self):
        bad = [
            b"not json",
            b"[1, 2]",
            b'{"repo": "no event"}',
            b'{"event": "x", "repo": 123}',
            b'{"event": "line\\nbreak"}',
            b'{"event": "' + b"a" * 201 + b'"}',
            b'{"event": "x", "user": "nel\\u0085break"}',
            b'{"event": "x", "user": "line\\u2028sep"}',
            b'{"event": "x", "user": "para\\u2029sep"}',
            b'{"event": "x", "user": "lone\\ud800"}',
        ]
        for body in bad:
            with self.subTest(body=body[:30]):
                r = self.client.post("/api/events", content=body, headers=AUTH)
                self.assertEqual(r.status_code, 400)
                self.assertFalse(r.json()["ok"])

    def test_japanese_text_and_remote_accepted(self):
        payload = {"event": "post-commit", "user": "山田　太郎", "remote": "github.com/example-org/game"}
        r = self.client.post("/api/events", json=payload, headers=AUTH)
        self.assertEqual(r.status_code, 201)
        event = self.client.get("/api/events", headers=AUTH).json()["events"][0]
        self.assertEqual(event["user"], "山田　太郎")
        self.assertEqual(event["remote"], "github.com/example-org/game")

    def test_body_too_large(self):
        body = b'{"event": "' + b"a" * MAX_BODY_BYTES + b'"}'
        r = self.client.post("/api/events", content=body, headers=AUTH)
        self.assertEqual(r.status_code, 413)


if __name__ == "__main__":
    unittest.main()
