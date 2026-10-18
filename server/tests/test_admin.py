"""管理画面のテスト（Discord にはつながない）

実行: .venv/bin/python -m unittest discover -s tests -v
"""

import json
import re
import sys
import unittest
from pathlib import Path
from unittest.mock import AsyncMock, patch

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
sys.path.insert(0, str(Path(__file__).resolve().parent))

from repo_tweaks_server.admin import MAX_FORM_BYTES  # noqa: E402
from repo_tweaks_server.notify import is_valid_webhook, mask_webhook  # noqa: E402
from repo_tweaks_server.security import verify_password  # noqa: E402
from test_app import AUTH, make_client  # noqa: E402

WEBHOOK = "https://discord.com/api/webhooks/123456789/abcdefghijklmnopqrstuvwxyz"


class FakeNotifier:
    def __init__(self):
        self.sent = []

    async def send(self, text):
        self.sent.append(text)
        return True, "送信しました"


def csrf_of(html):
    return re.search(r'name="csrf" value="([^"]+)"', html).group(1)


class AdminTestCase(unittest.TestCase):
    def setUp(self):
        # Wrong passwords wait 1 second; skip that in tests
        patcher = patch("repo_tweaks_server.admin.asyncio.sleep", new=AsyncMock())
        patcher.start()
        self.addCleanup(patcher.stop)
        self.notifier = FakeNotifier()
        self.client = make_client(notifier=self.notifier)

    def login(self, password="admin", client=None):
        client = client or self.client
        csrf = csrf_of(client.get("/login").text)
        return client.post("/login", data={"csrf": csrf, "password": password})

    def page(self):
        r = self.client.get("/")
        self.assertEqual(r.status_code, 200)
        return r.text

    def post(self, path, data):
        return self.client.post(path, data={"csrf": csrf_of(self.page()), **data})


class LoginTest(AdminTestCase):
    def test_pages_require_login(self):
        r = self.client.get("/")
        self.assertEqual((r.status_code, r.headers["location"]), (303, "/login"))
        self.assertEqual(self.client.post("/webhook", data={"webhook_url": WEBHOOK}).status_code, 401)

    def test_login_with_default_password_warns(self):
        r = self.login()
        self.assertEqual((r.status_code, r.headers["location"]), (303, "/"))
        self.assertIn("初期値（admin）のまま", self.page())

    def test_wrong_password(self):
        r = self.login("wrong")
        self.assertEqual(r.headers["location"], "/login?error=password")
        self.assertEqual(self.client.get("/").status_code, 303)

    def test_lockout_after_three_failures_and_notifies(self):
        for _ in range(3):
            r = self.login("wrong")
        self.assertEqual(r.headers["location"], "/login?error=locked")
        self.assertEqual(len(self.notifier.sent), 1)
        self.assertIn("🔒", self.notifier.sent[0])
        # Even the right password is refused while locked
        self.assertEqual(self.login().headers["location"], "/login?error=locked")
        self.assertIn("しばらくログインを止めています", self.client.get("/login").text)

    def test_login_form_needs_its_csrf_cookie(self):
        self.client.get("/login")
        r = self.client.post("/login", data={"csrf": "0" * 48, "password": "admin"})
        self.assertEqual(r.headers["location"], "/login?error=expired")

    def test_large_login_form_is_refused_before_parsing(self):
        csrf = csrf_of(self.client.get("/login").text)
        r = self.client.post("/login", data={"csrf": csrf, "password": "x" * (MAX_FORM_BYTES + 1)})
        self.assertEqual(r.status_code, 413)

    def test_damaged_password_record_does_not_crash(self):
        self.client.app.state.store.update(lambda s: s.update({"password": {"scheme": "scrypt", "salt": "zz"}}))
        with self.assertLogs("repo_tweaks", "WARNING"):
            self.assertEqual(self.login().headers["location"], "/login?error=password")
        with self.assertLogs("repo_tweaks", "WARNING"):
            self.assertFalse(verify_password("admin", {"scheme": "scrypt"}))
        self.assertFalse(verify_password("admin", "not a dict"))

    def test_logout(self):
        self.login()
        self.post("/logout", {})
        self.assertEqual(self.client.get("/").status_code, 303)


class FormProtectionTest(AdminTestCase):
    def setUp(self):
        super().setUp()
        self.login()

    def test_post_without_csrf_is_rejected(self):
        r = self.client.post("/webhook", data={"webhook_url": WEBHOOK})
        self.assertEqual(r.status_code, 403)

    def test_post_from_other_origin_is_rejected(self):
        csrf = csrf_of(self.page())
        r = self.client.post("/webhook", data={"csrf": csrf, "webhook_url": WEBHOOK},
                             headers={"Origin": "http://evil.example"})
        self.assertEqual(r.status_code, 403)

    def test_https_proxy_origin_with_same_host_is_accepted(self):
        # Behind an HTTPS reverse proxy the browser sends https:// while the server sees http://
        csrf = csrf_of(self.page())
        r = self.client.post("/webhook", data={"csrf": csrf, "webhook_url": WEBHOOK},
                             headers={"Origin": "https://testserver"})
        self.assertEqual(r.status_code, 303)

    def test_large_form_is_refused(self):
        csrf = csrf_of(self.page())
        r = self.client.post("/webhook", data={"csrf": csrf, "webhook_url": "x" * (MAX_FORM_BYTES + 1)})
        self.assertEqual(r.status_code, 413)

    def test_security_headers(self):
        for path in ("/", "/login", "/health"):
            headers = self.client.get(path).headers
            self.assertIn("frame-ancestors 'none'", headers["content-security-policy"])
            self.assertEqual(headers["x-frame-options"], "DENY")
            self.assertEqual(headers["referrer-policy"], "same-origin")


class WebhookTest(AdminTestCase):
    def setUp(self):
        super().setUp()
        self.login()

    def saved_url(self):
        return self.client.app.state.store.load()["webhook_url"]

    def test_rejects_non_discord_url(self):
        self.post("/webhook", {"webhook_url": "https://example.com/hook"})
        self.assertEqual(self.saved_url(), "")
        self.assertIn("Discord の Webhook URL", self.page())

    def test_save_masks_test_and_clear(self):
        self.post("/webhook", {"webhook_url": WEBHOOK})
        self.assertEqual(self.saved_url(), WEBHOOK)
        html = self.page()
        self.assertIn(mask_webhook(WEBHOOK), html)
        self.assertNotIn(WEBHOOK, html)

        self.post("/webhook/test", {})
        self.assertEqual(len(self.notifier.sent), 1)
        self.assertIn("テスト送信", self.page())

        self.post("/webhook", {"clear": "1"})
        self.assertEqual(self.saved_url(), "")

    def test_url_rules(self):
        self.assertTrue(is_valid_webhook(WEBHOOK))
        self.assertFalse(is_valid_webhook("http://discord.com/api/webhooks/1/x"))
        self.assertFalse(is_valid_webhook("https://discord.com.evil.example/api/webhooks/1/x"))
        self.assertFalse(is_valid_webhook(WEBHOOK + " x"))


class PasswordTest(AdminTestCase):
    def setUp(self):
        super().setUp()
        self.login()

    def change(self, current, new, confirm=None):
        self.post("/password", {"current": current, "new": new, "confirm": new if confirm is None else confirm})
        return self.client.app.state.store.load()

    def test_rules(self):
        cases = [
            ("wrong", "longenough1", None, "今のパスワードが違います"),
            ("admin", "short", None, "8 文字以上"),
            ("admin", "admin", None, "初期パスワードと同じ"),
            ("admin", "longenough1", "different1", "一致しません"),
        ]
        for current, new, confirm, message in cases:
            with self.subTest(message=message):
                settings = self.change(current, new, confirm)
                self.assertTrue(settings["password_is_default"])
                self.assertIn(message, self.page())

    def test_change_logs_out_other_browsers(self):
        second = self.client.__class__(self.client.app, follow_redirects=False)
        self.login(client=second)
        settings = self.change("admin", "new-password-1")
        self.assertFalse(settings["password_is_default"])
        self.assertNotIn("new-password-1", json.dumps(settings))
        self.assertNotIn("初期値（admin）のまま", self.page())
        self.assertEqual(second.get("/").status_code, 303)
        # The new password works; the old one does not
        third = self.client.__class__(self.client.app, follow_redirects=False)
        self.assertEqual(self.login("admin", client=third).headers["location"], "/login?error=password")
        self.assertEqual(self.login("new-password-1", client=third).headers["location"], "/")

    def test_password_survives_restart(self):
        self.change("admin", "new-password-1")
        restarted = make_client(data_dir=self.client.tmp.name)
        self.assertEqual(self.login("new-password-1", client=restarted).headers["location"], "/")


class RecentEventsTest(AdminTestCase):
    def test_index_lists_received_events(self):
        self.client.post("/api/events", json={"event": "post-commit", "remote": "github.com/example-org/game",
                                              "branch": "main", "user": "alice"}, headers=AUTH)
        self.login()
        html = self.page()
        self.assertIn("github.com/example-org/game", html)
        self.assertIn("post-commit", html)

    def test_event_text_is_escaped(self):
        self.client.post("/api/events", json={"event": "<script>x</script>"}, headers=AUTH)
        self.login()
        html = self.page()
        self.assertNotIn("<script>x</script>", html)
        self.assertIn("&lt;script&gt;", html)


if __name__ == "__main__":
    unittest.main()
