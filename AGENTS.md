# repo-tweaks

Git LFS のロックを土台にした、Unity チーム開発向けのファイルロック管理ツール。
ロックの正本は GitHub / GitLab の LFS ロック。自作サーバーは通知とキャッシュだけを担い、落ちてもロックは動く。

## Language

- Always communicate with the user in Japanese.
- Explanations, questions, progress updates, and summaries must be written in Japanese.
- Keep code, identifiers, commands, file names, and quoted error messages in their original language when appropriate.
- Even if source files, tool outputs, logs, or documentation are written in English, continue communicating with the user in Japanese.

## 進め方

- 小さく1段ずつ作る。全体の計画を一度に出さず、次の1段の範囲を利用者と決めてから実装する。
- 完了: Step 0（CLI ↔ サーバーの疎通テスト）、Step 1（`repo-tweaks list`。読むだけ）、Step 2（`lock` / `unlock`）、Step 3（サーバーの管理画面：ログイン・パスワード・Discord の通知先。NAS で起動・確認済み）、Step 4（lock / unlock の成功を CLI → サーバー → Discord に通知）、Step 5（サーバーによる定期的な照合と滞留の警告。GitHub のみ）、Step 6（`.gitattributes` の lockable を対象に、`status` / pre-commit の `check` / `install-hooks`。実機の確認待ち）。
- GUI：Step 0（Tauri の雛形と、ロック一覧の表示。読むだけ）。Mac / Windows で確認済み。Step 1（ロック・解除・強制解除、フォルダのドラッグ＆ドロップ）を作成、実機では未確認。
- 次の候補は、Unity 拡張（HANDOFF のフェーズ 2。ロックしていない lockable のアセットを `IsOpenForEdit` でグレーアウトし、ロック・解除のボタンを付ける）、GitLab への照合の対応。通知先は Discord（Slack は後回し）。
- 実機の確認（Windows、Fork、GitHub / GitLab への接続、NAS）は利用者が行う。結果を受け取ってから直す。

## 実機で確かめたこと（GitHub、Mac / Windows）

- Step 1・2 の list / lock / unlock は Mac と Windows の両方で動く（日本語の表示も文字化けしない）。別アカウントのロックは「他人のロック」に出る。
- Windows で大文字・小文字の違うパス（`readme.md`）を指定しても、`README.md` として扱われ、他人のロックを正しく検出する。
- **他人のロックの強制解除には、GitHub ではリポジトリの admin 権限が必要。** 書き込み権限だけでは `You must have admin access to force delete a lock` で拒否される（2026-10-18 確認）。GitLab は未確認。
- **GitHub では、自分のロックでも `git lfs unlock --force` は admin 権限が要る**（Windows で確認）。そのため、自分のロックの `--force` は `git lfs unlock --id`（`--force` なし）で外す。git-lfs 3.7 の `--id` は未コミットの変更の確認結果を使っていないので通る（将来の git-lfs で変わる可能性あり）。
- Step 4 の通知は Mac と Windows（別アカウント）の両方から Discord に届く。名前は LFS のユーザー名になり、`.meta` は元のファイルの行にまとまる（2026-10-18 確認）。
- GitHub は `git lfs lock` の応答に owner を入れない。通知の名前は、ロックの一覧の ours の owner から引き直す（通知するときだけ `git lfs locks` を1回多く呼ぶ）。
- 未コミットの変更があると、自分のロックでも Git LFS が解除を拒む（`Cannot unlock file with uncommitted changes`）。
- サーバー 0.2.0 は NAS の Docker で動き、Mac のブラウザから管理画面に入れる。Discord の Webhook へのテスト送信、CLI の `ping` も成功（2026-10-18 確認）。
- サーバー 0.4.1 は NAS から GitHub の LFS ロック API に届き、照合が動く。Fork で取ったロックは「確認待ち」になり、約 2 分後に「ロックを見つけました」が Discord に届く（2026-10-18 確認）。
- 照合のトークンは、Fine-grained トークン（対象のリポジトリだけ、Contents: Read-only）で足りる（2026-10-18 確認）。
- GUI Step 0 は Mac と Windows の両方でロック一覧を表示できる（別アカウントのロックは「他人のロック」に出る）。Windows で更新してもコンソールの窓は出ない。Mac で Finder から起動しても git-lfs が見つかる。メモリは Mac（開発モード）で本体と WebKit のプロセスの合計が約 112MB、Windows で本体と WebView2 の合計が約 66MB（2026-10-19 確認）。

## 言語・書き方

- **利用者への返答・説明・コードレビューの指摘は日本語で書く。** 英語で書いたら日本語に直す。
- 利用者向けの文面（CLI の出力・エラー、サーバーのエラー、README）は日本語。コードのコメントは英語で短く。

## 構成

```text
repo-tweaks/
├─ Cargo.toml        Rust workspace（members: core, cli）
├─ core/             Rust のライブラリ。git.rs（git の呼び出し・設定）、locks.rs（ロックの解釈・古さ・日本時間の表示）、lfs.rs（git lfs locks の実行とエラーの見分け）、paths.rs（パスをリポジトリのルートからの相対に直す）、ops.rs（ロック・解除の計画と実行）、lockable.rs（lockable のファイルの状態とコミット前の確認）
├─ cli/              Rust の CLI。コマンド名 repo-tweaks（list / lock / unlock / status / check / install-hooks / ping / notify / events）
├─ gui/              Tauri v2 + React + TypeScript + Tailwind CSS（shadcn/ui 風の部品は src/components/ui に手で置く）。src-tauri/ が Rust 側で、core を直接使う
├─ server/           Python 3.12 + Starlette + uvicorn + Jinja2。Docker で動かす。app.py（CLI 用 API）、admin.py（管理画面）、security.py / settings.py / notify.py（cloud-drive-sync の管理画面と同じ作り）
├─ hooks/            git hook（pre-commit / post-commit）。install-hooks が include_str! で埋め込んで書き出す
└─ unity/            Unity Editor 拡張（未着手）
```

- CLI の設定は環境変数 `REPO_TWEAKS_SERVER` / `REPO_TWEAKS_TOKEN`、無ければ git config `repo-tweaks.server` / `repo-tweaks.token`。
- サーバーは `/api/` に Bearer トークン（`REPO_TWEAKS_TOKEN`、32文字以上）が必須。ホスト名は `REPO_TWEAKS_ALLOWED_HOSTS` に限る（`*` 不可）。
- hook は git 操作を止めない（失敗しても終了コード 0、タイムアウト 3 秒）。
- `hooks/post-commit` は `git lfs install` の post-commit を置き換えるため、`git lfs post-commit` の行を必ず残す（消すと lockable の読み取り専用化が効かなくなる）。
- リポジトリの識別子は `remote.origin.url` を正規化した `host/owner/repo`（認証情報・ポートは除く）。フォルダ名に頼らない。
- ロジックは core に置き、CLI は表示と引数だけにする（GUI・Unity からも使うため）。
- GUI は CLI を呼ばず、core を直接使う。core の関数はカレントディレクトリを相手にするので、GUI 用に `GitConfig::read_in` / `RepoContext::open` / `lfs::fetch_locks_in` のようにフォルダを受け取る版を足す（GUI でカレントディレクトリを変えない。スレッドをまたいで共有されるため）。
- GUI の見出しのリポジトリ名は `git::repo_name`（remote の名前を優先、無ければフォルダ名）。CLI・サーバーと同じ名前にする。
- `git::run_in` は、git を起動する前にフォルダを確かめる（無い・フォルダでない・権限が無いを、「git が見つかりません」と取り違えない）。`RepoContext` の rev-parse が失敗したときは、git の理由の行（`fatal:` の行を優先）を添える（NAS 上の `dubious ownership` には `safe.directory` の案内）。
- GUI は起動時に `git::set_timeout(60 秒)` を呼ぶ。1つの git が 60 秒を超えたら、git が起動したもの（git-lfs・ssh）ごと止める（Unix は setsid で新しいセッションにしてプロセスグループごと、Windows は `taskkill /T`）。setsid で端末から切り離すので、ssh もパスフレーズやホスト鍵を端末で聞かずに失敗する。CLI では呼ばない（端末で認証を聞けるように）。
- GUI は `GIT_TERMINAL_PROMPT=0` も設定する。git lfs の呼び出しは GUI 全体で1本ずつ（Rust 側の Mutex）。更新に失敗しても直前の一覧は残す。最後に開いたフォルダは、開けたときだけ覚える。起動時の最初の取得は1回だけ（開発時の StrictMode でも）。
- GUI のロック・解除は core の `ops::plan_lock_in` / `plan_unlock_in`（フォルダを受け取る版）で、`.meta` は常に一緒。結果の文言は core の `Outcome::describe`（CLI と共通）。強制解除は「`preview_force_unlock` で外れる他人のロックを見せる → 確認 → `unlock_files` で計画を作り直し、他人のロックがすべて確認した ID のうちにあるときだけ実行」。自分のロックが Git LFS に断られたとき（未コミットの変更・手元に無い）は、確認してから `force`（ID で解除）。GUI はまだサーバーに通知しない（照合が拾う）。
- GUI の「N日以上ロックされたまま」の文は core の `locks::stale_warning`、更新の時刻も Rust 側の format_jst（日本時間）で作る。
- GUI の表示用の文字列（日本時間・経過時間・古さ）は Rust 側（gui/src-tauri/src/view.rs）で core の format_jst / format_age を使って作り、CLI と同じ言い回しにする。
- GUI は起動時に `git::hide_console_windows()` を呼ぶ（Windows で git の窓を出さない。CLI では呼ばない。端末で認証を聞けるようにするため）。Mac では起動時に Homebrew などの bin を PATH の先頭に足す（端末と同じ git を使う）。
- `gui/src-tauri` は workspace の外（`exclude`）で、Cargo.lock も別。ルートの `cargo test` には入らない。
- `.gitignore` の Python 用の `lib/` が `gui/src/lib` を隠すので、`!gui/src/lib/` で戻している。
- サーバーの security.py / settings.py / notify.py と admin.py のログイン・CSRF・ヘッダーの処理は、cloud-drive-sync の `cds_admin` の写し。安全のための修正をしたら、もう片方にも入れるよう利用者に伝える（このリポジトリで足した修正：フォームの大きさの上限、Origin はホスト名とポートで比較、scrypt を別スレッドで実行、ロックの通知を応答のあとに送る、壊れた admin.json で 500 にしない）。
- 管理画面の設定の既定の保存先は `server/data/dev`（起動した場所によらない。git に入れない）。
- `git lfs locks --verify --json` は、リポジトリ外・remote なし・認証失敗でも終了コード 0 で空の一覧を返す。必ず事前の確認と stderr（`fatal:` / `error`）の確認をして、空の一覧を「ロックなし」と誤表示しない。JSON の `[]`（`--verify` なしの形）も受け付けない。
- **ネットワークのエラー（ホストが無い等）では、`--json` は stderr に何も出さず、終了コード 0 で空の一覧を返す**（git-lfs 3.7 で確認）。そのため空の一覧のときだけ、`git lfs locks --verify --limit 1`（`--json` なし。エラーなら終了コード 2）でもう一度確かめる。
- lock / unlock は「計画（パスの解決・.meta の追加・一覧から自分 / 他人 / 未ロックを判定）→ 実行」の2段階。強制解除の確認は計画と実行の間で、呼ぶ側（CLI の y/N、Unity・GUI の画面）が行う。core は確認しない。
- `.meta` の扱い：lock はディスクにある相方を追加、unlock は自分の（`--force` なら誰のでも）ロックの相方だけを追加。相方は元のファイルが成功したときだけ処理する（持ち主が分かれないように）。`--no-meta` で無効。
- git lfs は1回の呼び出しに複数パスを渡す（指定したファイル → 相方の2回）。並列に実行しない（`.git/lfs/lockcache.db` を共有するため）。
- パスは `git ls-files` の `:(icase,literal)` で git の表記に合わせ、ロックとは大文字・小文字を無視して突き合わせる（大小違いで二重ロックしない）。
- 解除はパスで行う（Git LFS の「未コミットの変更があれば拒否」を生かす）。git lfs の出力形式は git-lfs 3.7 の `commands/command_lock.go` / `command_unlock.go` に合わせている。
- 時刻は表示が日本時間（固定 +09:00）、`--json` は UTC。古さの既定は 3 日、git config `repo-tweaks.staleDays` か `--stale-days` で変更（1〜365）。
- 通知：CLI は lock / unlock の結果から `lock` / `unlock` / `force_unlock` のイベント（`files`）を作ってサーバーへ送る。送れなくても終了コードは変えない（最初の失敗で残りは送らない）。サーバー未設定なら送らずに1行知らせ（`repo-tweaks.notify false` で消える）、トークンだけ無いときは警告する。`--no-notify` / `repo-tweaks.notify false` で止める。`user` は LFS サーバーの名前（`git lfs lock --json` の owner、または自分のロックの owner）を優先し、無ければ git の user.name。
- サーバーはイベントをメモリに最大 1000 件、1 件あたりパスは先頭 20 件だけ持つ（全体の数は `file_count`）。mem_limit 128m に収めるため。
- Discord の文面は server/repo_tweaks_server/messages.py（1操作1メッセージ、.meta は持ち主が同じときだけまとめる、1800 文字の予算で行を足し残りは「ほか N 件」）。利用者が決める文字列（パス・名前）は書式として解釈させず、送信時は `allowed_mentions: {"parse": []}` でメンションを止める。投稿は応答のあと（BackgroundTask）。
- 照合（server/repo_tweaks_server/reconcile.py）：前回のロック一覧と比べ、増えた・消えたもののうち通知済みでないものを流す。通知済みは「remote + ロックの ID」で state.json に残し、CLI のイベント（`files` の `id`）も記録する。初回は数えるだけ。失敗は「失敗し始め」と「直った」ときだけ通知（リポジトリごとにどんな例外も失敗として記録し、ほかは続ける）。通知済みの ID は、照合で見ていないものは 7 日で忘れる。取られて 2 分以内のロックは `pending` として持ち越し、2 分後にもう一度照合する（CLI の通知との行き違いを防ぐ。いつもの間隔は待たない）。remote は小文字にそろえる。照合中に登録が消えたら結果を保存しない。滞留の警告は照合が失敗中のリポジトリを除く。
- GitHub の LFS ロック API（server/repo_tweaks_server/lfs_api.py）：`https://github.com/<o>/<r>.git/info/lfs/locks`、Basic 認証。送り先は github.com だけ、リダイレクトとプロキシは使わない（トークンをよそへ送らない）。
- `notify` は hook の中で動くので、git の起動回数を増やさない（`git config --get-regexp` と `git rev-parse` を1回ずつ）。
- lockable（Step 6）：対象は `.gitattributes` の `lockable`（LFS で管理していないファイルにも効き、checkout で読み取り専用になる。git-lfs 3.7 で確認）。git には pathspec `:(attr:lockable)` で絞って聞く（`ls-files` / `status` / `diff` で使える）。`check` はステージされた変更・削除に自分のロックを求め、追加は求めない（ただし他人が先にそのパスをロックしていれば止める。`--no-renames` なので、リネーム元は削除として扱う）。マージ中（MERGE_HEAD がある）は、MERGE_HEAD とも違うファイルだけを見る。対象の変更が無ければネットワークに触れない。ロックを確かめられないときは警告して通す。
- pre-commit の hook は、`repo-tweaks check` の終了コードが 1 のときだけコミットを止める（2 などは古い repo-tweaks で `check` が無いときなので、警告して通す）。
- `git lfs install` は既にある hook（repo-tweaks の post-commit など）で止まり、そのあとの hook（post-merge など）を入れない。`install-hooks` は pre-push / post-checkout / post-merge が無ければ git-lfs 3.7 と同じ中身で書く。既にある hook は、読めない・UTF-8 でないものも含めて「別の hook」として扱い、黙って上書きしない。`core.hooksPath` が git 全体の設定なら入れない（ほかのリポジトリにも効くため）。
- Git LFS の pre-push のロック確認は、送る LFS のオブジェクトについてだけ。LFS 外の lockable（テキストの .unity など）は pre-commit の `check` だけで守る。

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
cd gui && npm run build && (cd src-tauri && cargo test)
cd gui && npm run tauri dev   # GUI を開発モードで起動
cd server && .venv/bin/python -m unittest discover -s tests -v
cd server && docker compose up -d --build   # .env が必要（.env.example をコピー）
```
