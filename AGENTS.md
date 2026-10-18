# repo-tweaks

Git LFS のロックを土台にした、Unity チーム開発向けのファイルロック管理ツール。
ロックの正本は GitHub / GitLab の LFS ロック。自作サーバーは通知とキャッシュだけを担い、落ちてもロックは動く。

## 進め方

- 小さく1段ずつ作る。全体の計画を一度に出さず、次の1段の範囲を利用者と決めてから実装する。
- 完了: Step 0（CLI ↔ サーバーの疎通テスト）、Step 1（`repo-tweaks list`。読むだけ）、Step 2（`lock` / `unlock`）、Step 3（サーバーの管理画面：ログイン・パスワード・Discord の通知先。NAS で起動・確認済み）。
- 次は、ロック・解除の成功を CLI からサーバーへ送り、Discord に通知する。通知先は Discord（Slack は後回し）。その後 `status`（`.gitattributes` の lockable と突き合わせ）。
- 実機の確認（Windows、Fork、GitHub / GitLab への接続、NAS）は利用者が行う。結果を受け取ってから直す。

## 実機で確かめたこと（GitHub、Mac / Windows）

- Step 1・2 の list / lock / unlock は Mac と Windows の両方で動く（日本語の表示も文字化けしない）。別アカウントのロックは「他人のロック」に出る。
- Windows で大文字・小文字の違うパス（`readme.md`）を指定しても、`README.md` として扱われ、他人のロックを正しく検出する。
- **他人のロックの強制解除には、GitHub ではリポジトリの admin 権限が必要。** 書き込み権限だけでは `You must have admin access to force delete a lock` で拒否される（2026-10-18 確認）。GitLab は未確認。
- 未コミットの変更があると、自分のロックでも Git LFS が解除を拒む（`Cannot unlock file with uncommitted changes`）。
- サーバー 0.2.0 は NAS の Docker で動き、Mac のブラウザから管理画面に入れる。Discord の Webhook へのテスト送信、CLI の `ping` も成功（2026-10-18 確認）。

## 言語・書き方

- **利用者への返答・説明・コードレビューの指摘は日本語で書く。** 英語で書いたら日本語に直す。
- 利用者向けの文面（CLI の出力・エラー、サーバーのエラー、README）は日本語。コードのコメントは英語で短く。

## 構成

```text
repo-tweaks/
├─ Cargo.toml        Rust workspace（members: core, cli）
├─ core/             Rust のライブラリ。git.rs（git の呼び出し・設定）、locks.rs（ロックの解釈・古さ・日本時間の表示）、lfs.rs（git lfs locks の実行とエラーの見分け）、paths.rs（パスをリポジトリのルートからの相対に直す）、ops.rs（ロック・解除の計画と実行）
├─ cli/              Rust の CLI。コマンド名 repo-tweaks（list / lock / unlock / ping / notify / events）
├─ server/           Python 3.12 + Starlette + uvicorn + Jinja2。Docker で動かす。app.py（CLI 用 API）、admin.py（管理画面）、security.py / settings.py / notify.py（cloud-drive-sync の管理画面と同じ作り）
├─ hooks/            git hook のサンプル（post-commit）
└─ unity/            Unity Editor 拡張（未着手）
```

- CLI の設定は環境変数 `REPO_TWEAKS_SERVER` / `REPO_TWEAKS_TOKEN`、無ければ git config `repo-tweaks.server` / `repo-tweaks.token`。
- サーバーは `/api/` に Bearer トークン（`REPO_TWEAKS_TOKEN`、32文字以上）が必須。ホスト名は `REPO_TWEAKS_ALLOWED_HOSTS` に限る（`*` 不可）。
- hook は git 操作を止めない（失敗しても終了コード 0、タイムアウト 3 秒）。
- `hooks/post-commit` は `git lfs install` の post-commit を置き換えるため、`git lfs post-commit` の行を必ず残す（消すと lockable の読み取り専用化が効かなくなる）。
- リポジトリの識別子は `remote.origin.url` を正規化した `host/owner/repo`（認証情報・ポートは除く）。フォルダ名に頼らない。
- ロジックは core に置き、CLI は表示と引数だけにする（GUI・Unity からも使うため）。
- サーバーの security.py / settings.py / notify.py と admin.py のログイン・CSRF・ヘッダーの処理は、cloud-drive-sync の `cds_admin` の写し。安全のための修正をしたら、もう片方にも入れるよう利用者に伝える（このリポジトリで足した修正：フォームの大きさの上限、Origin はホスト名とポートで比較、scrypt を別スレッドで実行、ロックの通知を応答のあとに送る、壊れた admin.json で 500 にしない）。
- 管理画面の設定の既定の保存先は `server/data/dev`（起動した場所によらない。git に入れない）。
- `git lfs locks --verify --json` は、リポジトリ外・remote なし・認証失敗でも終了コード 0 で空の一覧を返す。必ず事前の確認と stderr（`fatal:` / `error`）の確認をして、空の一覧を「ロックなし」と誤表示しない。JSON の `[]`（`--verify` なしの形）も受け付けない。
- lock / unlock は「計画（パスの解決・.meta の追加・一覧から自分 / 他人 / 未ロックを判定）→ 実行」の2段階。強制解除の確認は計画と実行の間で、呼ぶ側（CLI の y/N、Unity・GUI の画面）が行う。core は確認しない。
- `.meta` の扱い：lock はディスクにある相方を追加、unlock は自分の（`--force` なら誰のでも）ロックの相方だけを追加。相方は元のファイルが成功したときだけ処理する（持ち主が分かれないように）。`--no-meta` で無効。
- git lfs は1回の呼び出しに複数パスを渡す（指定したファイル → 相方の2回）。並列に実行しない（`.git/lfs/lockcache.db` を共有するため）。
- パスは `git ls-files` の `:(icase,literal)` で git の表記に合わせ、ロックとは大文字・小文字を無視して突き合わせる（大小違いで二重ロックしない）。
- 解除はパスで行う（Git LFS の「未コミットの変更があれば拒否」を生かす）。git lfs の出力形式は git-lfs 3.7 の `commands/command_lock.go` / `command_unlock.go` に合わせている。
- 時刻は表示が日本時間（固定 +09:00）、`--json` は UTC。古さの既定は 3 日、git config `repo-tweaks.staleDays` か `--stale-days` で変更（1〜365）。
- `notify` は hook の中で動くので、git の起動回数を増やさない（`git config --get-regexp` と `git rev-parse` を1回ずつ）。

## 環境

- 開発は Mac、利用者は Windows も使う。サーバーは最終的に LAN 内の NAS の Docker で動かす。Mac では Docker（OrbStack など）で確認できる。
- ホスト側のポートは既定 `8107`（`.env` の `REPO_TWEAKS_PORT`）。コンテナ内は `8000`。CLI 用 API と管理画面は同じポート。
- 管理画面の設定（`admin.json`）は名前付きボリューム `repo-tweaks-data`（コンテナ内 `/data`）。Mac で直接動かすときは `server/data/dev`（git に入れない）。
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
