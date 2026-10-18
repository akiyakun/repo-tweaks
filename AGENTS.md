# repo-tweaks

Git LFS のロックを土台にした、Unity チーム開発向けのファイルロック管理ツール。
ロックの正本は GitHub / GitLab の LFS ロック。自作サーバーは通知とキャッシュだけを担い、落ちてもロックは動く。

## 進め方

- 小さく1段ずつ作る。全体の計画を一度に出さず、次の1段の範囲を利用者と決めてから実装する。
- 現在は **Step 0（CLI ↔ サーバーの疎通テスト）** が完了した段階。次は `git lfs locks --verify --json` の解釈と `repo-tweaks list` の予定。
- 実機の確認（Windows、Fork、GitHub / GitLab への接続、NAS）は利用者が行う。結果を受け取ってから直す。

## 言語・書き方

- **利用者への返答・説明・コードレビューの指摘は日本語で書く。** 英語で書いたら日本語に直す。
- 利用者向けの文面（CLI の出力・エラー、サーバーのエラー、README）は日本語。コードのコメントは英語で短く。

## 構成

```text
repo-tweaks/
├─ Cargo.toml        Rust workspace（members: cli）
├─ cli/              Rust の CLI。コマンド名 repo-tweaks（ping / notify / events）
├─ server/           Python 3.12 + Starlette + uvicorn。Docker で動かす
├─ hooks/            git hook のサンプル（post-commit）
└─ unity/            Unity Editor 拡張（未着手）
```

- CLI の設定は環境変数 `REPO_TWEAKS_SERVER` / `REPO_TWEAKS_TOKEN`、無ければ git config `repo-tweaks.server` / `repo-tweaks.token`。
- サーバーは `/api/` に Bearer トークン（`REPO_TWEAKS_TOKEN`、32文字以上）が必須。ホスト名は `REPO_TWEAKS_ALLOWED_HOSTS` に限る（`*` 不可）。
- hook は git 操作を止めない（失敗しても終了コード 0、タイムアウト 3 秒）。
- `hooks/post-commit` は `git lfs install` の post-commit を置き換えるため、`git lfs post-commit` の行を必ず残す（消すと lockable の読み取り専用化が効かなくなる）。
- リポジトリの識別子は `remote.origin.url` を正規化した `host/owner/repo`（認証情報・ポートは除く）。フォルダ名に頼らない。
- `notify` は hook の中で動くので、git の起動回数を増やさない（`git config --get-regexp` と `git rev-parse` を1回ずつ）。

## 環境

- 開発は Mac、利用者は Windows も使う。サーバーは最終的に LAN 内の NAS の Docker で動かす。Mac では Docker（OrbStack など）で確認できる。
- ホスト側のポートは既定 `8105`（`.env` の `REPO_TWEAKS_PORT`）。コンテナ内は `8000`。
- Docker は堅めに作る：`python:3.12-slim`、UID/GID `10001`、`read_only`、`cap_drop: [ALL]`、`no-new-privileges`、許可リスト方式の `.dockerignore`、秘密は `.env`（git に入れない）。
- 共有の `Cargo.toml` に、特定の環境だけの回避策を入れない。マシンごとの設定は git に入らない `.cargo/config.toml` に書く。
- リポジトリがネットワーク共有上にあると `core.fileMode=false` になることがある。実行ファイル（hook など）は `git add --chmod=+x` で実行権限を付けてコミットする。
- 利用者個人の環境（実際のパス、IP、機材、ほかのリポジトリとの関係）は `CLAUDE.local.md`（git に入らない）に書いてある。

## 個人情報・機密情報

- **コミットするファイルに、個人の環境の情報や秘密を書かない。** 次のものは `CLAUDE.local.md` や `.env` など git に入らない場所に書く。
  - 実際の絶対パス（`/Users/<名前>/...`、マウント先、NAS の共有フォルダのパスなど）
  - 実際の IP アドレス・ホスト名、機材の型番、マシン名
  - 個人の名前・ユーザー名・メールアドレス
  - トークン・パスワード・API キー・Webhook URL
- 例を書くときは、次のような架空の値を使う。
  - IP: `192.0.2.10`（文書用に予約された範囲 192.0.2.0/24）
  - ホスト名・URL: `example.com`、`github.com/example-org/game`
  - パス: `/path/to/repo-tweaks`、`/Users/<名前>/...`
  - 人: `alice`、`山田　太郎`
- コミットの前に、追加・変更するファイルにこれらが入っていないか確かめる。

## 確認コマンド

```bash
cargo test
cd server && .venv/bin/python -m unittest discover -s tests -v
cd server && docker compose up -d --build   # .env が必要（.env.example をコピー）
```
