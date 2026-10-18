"""GitHub の LFS ロック API からロックの一覧を取る（読むだけ）

- エンドポイント: https://github.com/<owner>/<repo>.git/info/lfs/locks（Git LFS の Locking API）
- 認証: Basic（GitHub のユーザー名と、トークン）
- 返すロックは {"id", "path", "owner", "locked_at"}。owner は名前の文字列にそろえる

送り先は github.com だけにする（登録された URL で、よその場所へ認証情報を送らせないため）。
"""

import base64
import json
import re
import urllib.error
import urllib.parse
import urllib.request

from . import __version__

ALLOWED_HOST = "github.com"
TIMEOUT_SECONDS = 15
PAGE_LIMIT = 100
MAX_PAGES = 50
_NAME = re.compile(r"^[A-Za-z0-9_.-]{1,100}$")


class LfsApiError(Exception):
    """利用者向けの日本語の理由を持つ（トークンなどは含めない）"""


def parse_repo(url):
    """GitHub のリポジトリの URL から "github.com/<owner>/<repo>" を作る。だめなら ValueError

    受け付ける形: https://github.com/o/r(.git)(/)、git@github.com:o/r.git、ssh://git@github.com/o/r.git
    CLI（cli/src/git.rs の normalize_remote）が送る remote と同じ形になる。
    GitHub は大文字・小文字を区別しないので、小文字にそろえる（CLI の remote も小文字にして突き合わせる）
    """
    text = (url or "").strip()
    if "://" in text:
        parts = urllib.parse.urlsplit(text)
        host, path = (parts.hostname or "").lower(), parts.path
    elif re.match(r"^[^/@]+@[^:/]+:", text):
        host, path = text.split("@", 1)[1].split(":", 1)
        host = host.lower()
    else:
        raise ValueError("GitHub のリポジトリの URL（https://github.com/<owner>/<repo>）を入れてください")
    if host != ALLOWED_HOST:
        raise ValueError("今は GitHub（github.com）のリポジトリだけを登録できます")
    path = path.strip("/")
    if path.endswith(".git"):
        path = path[:-4]
    segments = path.split("/")
    if len(segments) != 2 or not all(_NAME.match(s) for s in segments):
        raise ValueError("URL は https://github.com/<owner>/<repo> の形にしてください")
    return f"{ALLOWED_HOST}/{segments[0]}/{segments[1]}".lower()


def locks_url(remote):
    """"github.com/o/r" -> LFS の Locking API の URL"""
    host, owner, repo = remote.split("/")
    return f"https://{host}/{owner}/{repo}.git/info/lfs/locks"


def fetch_locks(remote, username, token):
    """そのリポジトリのロックをすべて取る。失敗したら LfsApiError"""
    auth = base64.b64encode(f"{username}:{token}".encode("utf-8")).decode("ascii")
    headers = {
        "Accept": "application/vnd.git-lfs+json",
        "Authorization": f"Basic {auth}",
        "User-Agent": f"repo-tweaks/{__version__}",
    }
    locks, cursor = [], ""
    for _ in range(MAX_PAGES):
        query = {"limit": PAGE_LIMIT}
        if cursor:
            query["cursor"] = cursor
        url = f"{locks_url(remote)}?{urllib.parse.urlencode(query)}"
        data = _get_json(url, headers)
        locks.extend(_lock(item) for item in data.get("locks") or [])
        cursor = data.get("next_cursor") or ""
        if not cursor:
            return locks
    raise LfsApiError(f"ロックが多すぎて取り切れませんでした（{PAGE_LIMIT * MAX_PAGES} 件まで）")


def _lock(item):
    owner = item.get("owner") or {}
    return {
        "id": str(item.get("id", "")),
        "path": str(item.get("path", "")),
        "owner": str(owner.get("name", "") if isinstance(owner, dict) else owner) or "（不明）",
        "locked_at": str(item.get("locked_at", "")),
    }


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    """Never follow redirects: they would carry the Authorization header to another place"""

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


# No proxies from the environment either; the token goes to github.com only
_OPENER = urllib.request.build_opener(_NoRedirect, urllib.request.ProxyHandler({}))


def _get_json(url, headers):
    request = urllib.request.Request(url, headers=headers, method="GET")
    try:
        with _OPENER.open(request, timeout=TIMEOUT_SECONDS) as response:
            return json.loads(response.read().decode("utf-8"))
    except urllib.error.HTTPError as e:
        raise LfsApiError(_http_reason(e.code)) from None
    except (urllib.error.URLError, TimeoutError, OSError):
        raise LfsApiError("GitHub につながりませんでした（NAS からインターネットに出られるか確かめてください）") from None
    except (ValueError, UnicodeDecodeError):
        raise LfsApiError("GitHub の応答を読めませんでした") from None


def _http_reason(code):
    if code == 401:
        return "GitHub にログインできませんでした（401）。ユーザー名とトークンを確かめてください"
    if code == 403:
        return "権限がありません（403）。トークンに、このリポジトリを読む権限があるか確かめてください"
    if code == 404:
        return "リポジトリが見つかりません（404）。URL と、トークンがこのリポジトリを読めるかを確かめてください"
    if 300 <= code < 400:
        return f"GitHub が別の場所へ案内しました（{code}）。リポジトリの名前が変わっていないか確かめてください"
    if code == 429:
        return "GitHub の回数制限にかかりました（429）。しばらくしてから試してください"
    return f"GitHub がエラーを返しました（{code}）"
