"""サーバーの設定（環境変数から起動時に1回だけ読む）"""

import os
from dataclasses import dataclass

MIN_TOKEN_LENGTH = 32


@dataclass(frozen=True)
class Settings:
    token: str
    allowed_hosts: tuple
    max_events: int = 100

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
        if not 1 <= max_events <= 10000:
            raise ValueError("REPO_TWEAKS_MAX_EVENTS は 1〜10000 の整数にしてください")
        return cls(token=token, allowed_hosts=hosts, max_events=max_events)
