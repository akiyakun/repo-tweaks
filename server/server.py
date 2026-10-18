"""repo-tweaks サーバーの入口（NAS のコンテナの入口）

使い方:
    python server.py                 # 0.0.0.0:8000 で待ち受ける（コンテナの中）
    python server.py --port 8105     # Mac で直接試すとき（REPO_TWEAKS_TOKEN が必要）
"""

import argparse
import logging
import sys

import uvicorn

from repo_tweaks_server.app import create_app
from repo_tweaks_server.config import Settings


def main():
    parser = argparse.ArgumentParser(description="repo-tweaks サーバー")
    parser.add_argument("--host", default="0.0.0.0")
    parser.add_argument("--port", type=int, default=8000)
    args = parser.parse_args()
    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s", stream=sys.stdout)
    try:
        settings = Settings.from_env()
    except ValueError as e:
        sys.exit(f"起動できません: {e}")
    uvicorn.run(create_app(settings), host=args.host, port=args.port, log_level="warning",
                server_header=False, proxy_headers=False, access_log=False)


if __name__ == "__main__":
    main()
