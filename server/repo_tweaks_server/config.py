"""サーバーの設定（環境変数から起動時に1回だけ読む）"""

import os
from dataclasses import dataclass
from pathlib import Path

MIN_TOKEN_LENGTH = 32
# Fixed to server/data/dev (git-ignored) wherever the server is started from
DEFAULT_DATA_DIR = str(Path(__file__).resolve().parent.parent / "data" / "dev")


@dataclass(frozen=True)
class Settings:
    token: str
    allowed_hosts: tuple
    max_events: int = 100
    # Where the admin screen keeps admin.json (password hash, webhook). /data in the container.
    data_dir: str = DEFAULT_DATA_DIR

    @classmethod
    def from_env(cls, env=None):
        env = os.environ if env is None else env
        token = env.get("REPO_TWEAKS_TOKEN", "")
        if len(token) < MIN_TOKEN_LENGTH:
            raise ValueError(f"REPO_TWEAKS_TOKEN を {MIN_TOKEN_LENGTH} 文字以上で設定してください")
        hosts = tuple(h.strip() for h in env.get("REPO_TWEAKS_ALLOWED_HOSTS", "localhost,127.0.0.1").split(",")
                      if h.strip())
        if not hosts or "*" in hosts:
            raise ValueError("REPO_TWEAKS_ALLOWED_HOSTS にはホスト名を書いてください（* は使えません）")
        try:
            max_events = int(env.get("REPO_TWEAKS_MAX_EVENTS", "100"))
        except ValueError:
            max_events = 0
        # Each kept event holds at most STORED_FILES paths, so 1000 events stay within the container's memory
        if not 1 <= max_events <= 1000:
            raise ValueError("REPO_TWEAKS_MAX_EVENTS は 1〜1000 の整数にしてください")
        data_dir = env.get("REPO_TWEAKS_DATA_DIR", "").strip() or DEFAULT_DATA_DIR
        return cls(token=token, allowed_hosts=hosts, max_events=max_events, data_dir=data_dir)
