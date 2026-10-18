# repo-tweaks

Git LFS のロックを土台にした、ファイルロック管理ツール（作成中）。
いまは **Step 0: CLI ↔ サーバーの疎通テスト** の段階です。

```text
git commit
  └─ .git/hooks/post-commit
       └─ repo-tweaks notify ──HTTP POST──▶ [Docker] repo-tweaks サーバー
                                              ├─ 受け取った内容をログに出す
                                              └─ GET /api/events で直近の通知を返す
```

| フォルダ | 中身 |
|---|---|
| `cli/` | Rust の CLI（コマンド名 `repo-tweaks`） |
| `server/` | Python（Starlette）のサーバー。Docker で動かす |
| `hooks/` | git hook のサンプル |
| `unity/` | Unity Editor 拡張（未着手） |

## 1. サーバーを起動する（Mac の OrbStack / NAS 共通）

```bash
cd server
cp .env.example .env
# .env の REPO_TWEAKS_TOKEN に 32 文字以上の値を入れる（例: openssl rand -hex 32 の結果）
docker compose up -d --build
docker compose logs -f        # 受け取った通知がここに出る
```

ホスト側のポートは `8105` です（`.env` の `REPO_TWEAKS_PORT`）。

## 2. CLI を入れて設定する

```bash
cargo install --path cli      # ~/.cargo/bin/repo-tweaks に入る
git config --global repo-tweaks.server http://localhost:8105
git config --global repo-tweaks.token <サーバーの .env と同じトークン>
repo-tweaks ping
```

設定は環境変数 `REPO_TWEAKS_SERVER` / `REPO_TWEAKS_TOKEN` が優先され、無ければ git config を読みます。
Fork などの GUI クライアントはシェルの設定（`.zshrc`）を読まないため、git config に書くのが確実です。

## 3. git hook から送る

試したいリポジトリで:

```bash
cp /path/to/repo-tweaks/hooks/post-commit .git/hooks/post-commit
chmod +x .git/hooks/post-commit
git commit --allow-empty -m "hook test"
repo-tweaks events            # サーバーが受け取った通知を確認
```

- このサンプルは、`git lfs install` が入れる post-commit（`git lfs post-commit`。lockable なファイルを読み取り専用に戻す処理）も呼びます。そのため、LFS の hook を上書きしてもかまいません。
- **LFS 以外の自作の post-commit が既にある場合は、上書きせずに** サンプルの `# --- repo-tweaks ---` 以降を既存の hook の末尾に足してください。
- この hook を入れたあとで `git lfs install` を実行すると「Hook already exists」と表示されますが、LFS の処理は入っているので問題ありません。
- サーバーが止まっていても、警告が出るだけでコミットは成功します（3 秒で打ち切り）。
- `repo-tweaks` が見つからないときは、警告を出して通知を飛ばします（`~/.cargo/bin` も探します。Windows では `.exe`）。

## コマンド

| コマンド | 内容 |
|---|---|
| `repo-tweaks ping` | サーバーにつながるか、トークンが正しいかを確かめる |
| `repo-tweaks notify --event <種類>` | 今のリポジトリ（名前と origin の URL）・ブランチ・コミット・ユーザー名を送る |
| `repo-tweaks events [--limit N]` | サーバーが受け取った通知を新しい順に表示する |

## サーバーの API

| メソッド・パス | 認証 | 内容 |
|---|---|---|
| `GET /health` | なし | 動いているかの確認 |
| `GET /api/ping` | Bearer | トークンを含めた疎通確認 |
| `POST /api/events` | Bearer | 通知を受け取る（本文は 16KB まで） |
| `GET /api/events` | Bearer | 直近の通知（メモリのみ。再起動で消える） |

## テスト

```bash
cargo test
cd server && python3 -m venv .venv && .venv/bin/python -m pip install -r requirements-dev.txt
.venv/bin/python -m unittest discover -s tests -v
```

リポジトリを SMB 共有などのネットワークドライブに置いている場合、インクリメンタルビルド用のロックファイルが作れずにビルドが失敗します。
その場合は、git に入らない `.cargo/config.toml` でビルドの出力先をローカルディスクに向けてください。

```toml
[build]
target-dir = "/Users/<名前>/.cache/cargo-target/repo-tweaks"
```

## 送る内容

`notify` が送るリポジトリの識別子 `remote` は、`remote.origin.url` から作ります。
認証情報（`https://user:token@...`）とポート、`.git` を取り除き、`github.com/owner/repo` の形にします。
clone したフォルダ名が人によって違っても、同じリポジトリとして扱えます。origin が無いときはフォルダ名だけを送ります。
