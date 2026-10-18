"""管理画面の設定（admin.json）の読み書き

admin.json に置くもの:
- password: パスワードのハッシュ（パスワードそのものは保存しない）
- password_is_default: 初期パスワード（admin）のままか
- webhook_url: Discord の Webhook URL
- lockout: ログインのロック（max_failures 回、window_seconds 秒以内に失敗したら lock_seconds 秒ロック）
- session_hours: ログインが続く時間
- repos: 照合するリポジトリ [{id, remote, username, token}]（token は画面では伏せ字）
- reconcile: 照合の間隔（interval_minutes）
- stale: 滞留の警告（enabled、days 日以上、毎日 time に送る。時刻は日本時間）
"""

import copy
import json
import os
import threading
from pathlib import Path

from .security import hash_password

DEFAULT_PASSWORD = "admin"

DEFAULTS = {
    "password": None,
    "password_is_default": True,
    "webhook_url": "",
    "lockout": {"max_failures": 3, "window_seconds": 180, "lock_seconds": 900},
    "session_hours": 12,
    "repos": [],
    "reconcile": {"interval_minutes": 60},
    "stale": {"enabled": True, "days": 3, "time": "10:00"},
}

MIN_INTERVAL, MAX_INTERVAL = 10, 1440
MIN_STALE_DAYS, MAX_STALE_DAYS = 1, 365


class SettingsStore:
    """admin.json を読み書きする。書き込みは一時ファイルに書いてから差し替える"""

    def __init__(self, path):
        self.path = Path(path)
        self._lock = threading.Lock()
        settings = self.load()
        if not settings["password"]:
            # First start: set the initial password (admin)
            settings["password"] = hash_password(DEFAULT_PASSWORD)
            settings["password_is_default"] = True
            self.save(settings)

    def load(self):
        with self._lock:
            data = json.loads(self.path.read_text(encoding="utf-8")) if self.path.exists() else {}
        settings = copy.deepcopy(DEFAULTS)
        for key, value in data.items():
            if isinstance(settings.get(key), dict) and isinstance(value, dict):
                settings[key].update(value)
            else:
                settings[key] = value
        return settings

    def save(self, settings):
        with self._lock:
            self.path.parent.mkdir(parents=True, exist_ok=True)
            tmp = self.path.with_name(f".{self.path.name}.tmp")
            tmp.write_text(json.dumps(settings, ensure_ascii=False, indent=1), encoding="utf-8")
            os.replace(tmp, self.path)

    def update(self, change):
        """change(settings) で書き換えて保存する。書き換えた設定を返す"""
        settings = self.load()
        change(settings)
        self.save(settings)
        return settings
