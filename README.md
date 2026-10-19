# repo-tweaks


Git LFS のロックを土台にした、ファイルロック管理ツール（作成中）。
いまは **Step 6: ロックが必要なファイルの指定（lockable）と、コミット前の確認** まで出来ています。

Step 0 で作った、CLI とサーバーの疎通の流れ：

```text
git commit
  └─ .git/hooks/post-commit
       └─ repo-tweaks notify ──HTTP POST──▶ [Docker] repo-tweaks サーバー
                                              ├─ 受け取った内容をログに出す
                                              └─ GET /api/events で直近の通知を返す
```

| フォルダ | 中身 |
|---|---|
| `core/` | Rust のライブラリ。git / git lfs の呼び出し、ロックの解釈と古さの判定、ロック・解除（CLI・GUI・Unity で共通に使う） |
| `cli/` | Rust の CLI（コマンド名 `repo-tweaks`） |
| `server/` | Python（Starlette）のサーバー。CLI からの通知の受け口と、管理画面。Docker で動かす |
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

ホスト側のポートは `8107` です（`.env` の `REPO_TWEAKS_PORT`）。
ブラウザで `http://localhost:8107/` を開くと管理画面です（初期パスワードは `admin`）。

### NAS で動かす

NAS の、このリポジトリのフォルダで実行します（SSH で入るか、UGOS の Docker の画面からプロジェクトとして）。

```bash
cd server
cp .env.example .env
```

`.env` を次のように書き換えます（IP は NAS の LAN の IP。例の `192.0.2.10` は架空の値です）。

```text
REPO_TWEAKS_TOKEN=<openssl rand -hex 32 などで作った値>
REPO_TWEAKS_BIND_IP=192.0.2.10
REPO_TWEAKS_ALLOWED_HOSTS=localhost,127.0.0.1,192.0.2.10
```

```bash
docker compose up -d --build
```

起動したら、ブラウザで `http://<NAS の IP>:8107/` を開き、次の順に設定します。

1. 初期パスワード `admin` でログインし、「パスワードの変更」で変える（変えるまで、画面の上部に警告が出続けます）
2. Discord で Webhook を作る（チャンネルの設定 → 連携サービス → ウェブフック → 新しいウェブフック → 「ウェブフック URL をコピー」）
3. 「通知（Discord）」に貼って保存し、「テスト送信」で Discord に届くか確かめる

CLI の送り先も NAS に変えます。

```bash
git config --global repo-tweaks.server http://<NAS の IP>:8107
git config --global repo-tweaks.token <NAS の .env と同じトークン>
repo-tweaks ping
```

## 管理画面

| 欄 | 内容 |
|---|---|
| 通知（Discord） | Webhook URL の保存（画面では伏せ字）・削除・テスト送信。CLI でのロック・解除・強制解除と、管理画面のログインのロックを通知します |
| 直近の受信 | CLI・git hook から受け取った通知の、新しい順に 20 件（メモリだけ。再起動で消えます） |
| ロックの照合（GitHub） | 照合するリポジトリの登録（URL・ユーザー名・トークン。トークンは伏せ字）・削除、前回の照合の結果、今すぐ照合、照合の間隔（10〜1440 分。既定 60 分） |
| 滞留の警告 | 有効・無効、何日以上（既定 3 日）、毎日何時に送るか（既定 10:00、日本時間） |
| パスワードの変更 | 8 文字以上。初期パスワード（admin）は使えません。変えると、ほかのブラウザのログインは切れます |

- 作りは cloud-drive-sync の管理画面と同じです。パスワードは scrypt のハッシュだけを保存します。**3 分以内に 3 回まちがえると 15 分ロック**し、Discord に通知します。
- ログインの状態はサーバーのメモリだけに持ちます（コンテナを再起動するとログアウトします。12 時間で切れます）。
- フォームには CSRF 対策の合言葉と Origin の確認、すべての応答に安全のためのヘッダー（CSP、`X-Frame-Options: DENY` など）を付けています。
- 設定（`admin.json`：パスワードのハッシュ・Webhook URL）は、Docker の名前付きボリューム `repo-tweaks-data` に保存します。コンテナを作り直しても残ります。
- 通信は暗号化しません（HTTP）。家・社内の LAN だけで使う前提です。HTTPS にしたいときは NAS のリバースプロキシなどを前に置けます（フォームの確認はホスト名とポートで行うので、プロキシが `Host` ヘッダーをそのまま渡す設定にしてください）。
- 管理画面のフォームは 8 KB まで受け付けます（それより大きい送信は、中身を読む前に断ります）。
- **パスワードを忘れたら**：設定を消して再起動すると、初期パスワード `admin` に戻ります（Webhook URL も消えるので、登録し直してください）。

  ```bash
  docker compose stop
  docker run --rm -v repo-tweaks-data:/data busybox rm /data/admin.json
  docker compose start
  ```

## 2. CLI を入れて設定する

```bash
cargo install --path cli      # ~/.cargo/bin/repo-tweaks に入る
git config --global repo-tweaks.server http://localhost:8107
git config --global repo-tweaks.token <サーバーの .env と同じトークン>
repo-tweaks ping
```

設定は環境変数 `REPO_TWEAKS_SERVER` / `REPO_TWEAKS_TOKEN` が優先され、無ければ git config を読みます。
Fork などの GUI クライアントはシェルの設定（`.zshrc`）を読まないため、git config に書くのが確実です。

## 3. git hook を入れる

使うリポジトリごとに、1 回だけ実行します（Mac / Windows 共通）。

```bash
git lfs install               # Git LFS の設定（初めてのときだけ。hook が既にあると止まることがあるが、気にしなくてよい）
repo-tweaks install-hooks     # repo-tweaks の pre-commit・post-commit と、足りない Git LFS の hook
git commit --allow-empty -m "hook test"
repo-tweaks events            # サーバーが受け取った通知を確認
```

| hook | 内容 |
|---|---|
| pre-commit | ロックが必要なファイルを、ロックせずに変更・削除していたらコミットを止める（下の「ロックが必要なファイル」） |
| post-commit | コミットをサーバーに知らせる。Git LFS の post-commit（lockable なファイルを読み取り専用に戻す処理）も呼ぶ |

- `install-hooks` は、git が使う hook のフォルダ（`core.hooksPath` を設定していればそちら）に書きます。
  - 同じ内容ならそのまま、以前の repo-tweaks の hook と Git LFS 標準の post-commit は置き換えます。
  - **それ以外の hook が既にあるときは書き換えません**。`--force` を付けると、元の hook を `<名前>.bak` に残してから上書きします。自作の hook を残したいときは、`hooks/` のサンプルの中身を既存の hook に足してください。
- Git LFS の hook（pre-push / post-checkout / post-merge）が無ければ、`git lfs install` と同じものを入れます（`git lfs install` は、既にある hook で止まり、そのあとの hook を入れないことがあるため）。別の hook があって Git LFS の処理が入っていないときは、警告だけ出します。
- `core.hooksPath` が git 全体の設定（`~/.gitconfig` など）になっているときは、入れません。入れると、ほかのすべてのリポジトリでもコミットの確認と通知が動いてしまうためです。
- post-commit は、サーバーが止まっていても警告が出るだけでコミットは成功します（3 秒で打ち切り）。
- `repo-tweaks` が見つからないときは、警告を出してそのまま進みます（`~/.cargo/bin` も探します。Windows では `.exe`）。Fork などの GUI からコミットするときも同じです。

## コマンド

| コマンド | 内容 |
|---|---|
| `repo-tweaks list [--json] [--stale-days N]` | ロックの一覧を、自分のロック / 他人のロックに分けて表示する。古いロックに印を付ける |
| `repo-tweaks lock <パス>... [--no-meta] [--json]` | ファイルをロックする。Unity の `.meta` があれば一緒にロックする |
| `repo-tweaks unlock <パス>... [--force [--yes]] [--no-meta] [--json]` | ロックを解除する。ロック中の `.meta` も一緒に解除する |
| `repo-tweaks status [--all] [--json]` | ロックが必要なファイル（lockable）の、ロックと変更の状態を表示する |
| `repo-tweaks check` | コミットしようとしている変更を確かめる（pre-commit hook から呼ぶ） |
| `repo-tweaks install-hooks [--force]` | このリポジトリに pre-commit と post-commit の hook を入れる |
| `repo-tweaks ping` | サーバーにつながるか、トークンが正しいかを確かめる |
| `repo-tweaks notify --event <種類>` | 今のリポジトリ（名前と origin の URL）・ブランチ・コミット・ユーザー名を送る |
| `repo-tweaks events [--limit N]` | サーバーが受け取った通知を新しい順に表示する |

## ロックの一覧（list）

Git のリポジトリの中で実行します。`git lfs locks --verify --json` で GitHub / GitLab からロックを取ってきて表示します（読むだけで、ロックは変えません）。

```text
自分のロック（1）
  afl/README.md             10/18 01:30 から  1日2時間

他人のロック（2）
  Assets/Scenes/Main.unity  bob   10/14 10:00 から  4日17時間  ⚠ 古い
  Assets/Art/hero.psd       山田  10/18 18:20 から  9時間

⚠ 3日以上ロックされたままのものが 1 件あります
```

- 時刻は日本時間です。一覧は、古いロックから順に並べます。
- 「古い」とみなす日数は、既定で 3 日です。`--stale-days N`、またはリポジトリごと・全体の git config で変えられます（1〜365）。

  ```bash
  git config --global repo-tweaks.staleDays 5
  ```

- `--json` は、ほかのツール（Unity 拡張や GUI）から使うための出力です。時刻は UTC です。

  ```json
  {"stale_days": 3, "ours": [{"id": "57707244", "path": "afl/README.md", "owner": "alice",
    "locked_at": "2026-10-17T16:30:58Z", "age_seconds": 93600, "stale": false}], "theirs": []}
  ```

- `git lfs locks` は、リポジトリの外、remote が無いとき、ログインに失敗したときでも、エラーにならずに空の一覧を返します。`list` はこれらを先に確かめるか、git lfs のエラー出力から見分けて、理由を表示します（「ロックはありません」と誤って表示しないため）。

## ロックの取得・解除（lock / unlock）

```text
$ repo-tweaks lock Assets/Scenes/Main.unity README.md Assets/Art/hero.psd
ロックしました  Assets/Scenes/Main.unity
ロックしました  Assets/Scenes/Main.unity.meta（自動で追加）
ロック済み      README.md                             自分がロック中
できません      Assets/Art/hero.psd                   bob さんが 10/14 10:00 からロック中
repo-tweaks: 1 件は処理できませんでした
```

- パスは、今いるフォルダからの相対パスか、絶対パスで指定します。Windows の `\`、Git Bash の `/c/...` も使えます。
- 大文字・小文字の違いは、git が管理している表記に合わせます（Mac / Windows で `assets/main.unity` と打っても `Assets/Main.unity` のロックになります）。別の表記で二重にロックしないためです。
- フォルダは指定できません。ファイルを指定してください。
- 先にロックの一覧を取り、自分 / 他人 / 未ロックを確かめてから、まとめて `git lfs lock` / `git lfs unlock` を実行します。
- 1件でも「できません」があれば、終了コードは 1 になります。「ロック済み」「ロックなし」は失敗に数えません。
- **`.meta`**：`Main.unity` をロックすると、`Main.unity.meta` があれば一緒にロックします（`.meta` を指定したときは、元のアセットも）。解除のときは、自分がロックしている相方だけを一緒に解除します（`--force` のときは他人のものも）。
  - 元のファイルをロック・解除できなかったときは、相方は「しませんでした」として触りません（アセットと `.meta` の持ち主が分かれないようにするため）。
  - `--no-meta` で止められます。
- **未コミットの変更**：自分のロックでも、未コミットの変更があるファイルは Git LFS が解除を拒みます。コミットしてから解除するか、`--force` を付けてください。削除・移動して手元に無いファイルの解除も `--force` が必要です。
  - 自分のロックに `--force` を付けたときは、ロックの ID で解除します（`git lfs unlock --id`）。GitHub では `git lfs unlock --force` は管理者だけができるので、管理者でなくても自分のロックを外せるようにするためです。
- **他人のロックの強制解除**：`--force` を付けると、次のように確認してから解除します。

  ```text
  次の他人のロックを強制解除します:
    Assets/Art/hero.psd  bob  10/14 10:00 から
  よろしいですか？ [y/N]:
  ```

  Unity や GUI から呼ぶときは、呼ぶ側の画面で確認したうえで `--yes` を付けます。入力できない状態で `--yes` が無いときは、何もしません。強制解除できるかどうかは、GitHub / GitLab での権限によります。**GitHub ではリポジトリの管理者（admin）権限が必要です**（書き込み権限だけでは「You must have admin access to force delete a lock」で断られます）。GitLab は未確認です。
- `--json` は、ファイルごとの結果（`result`：`locked` / `already_ours` / `held_by_other` / `no_such_file` / `is_directory` / `unlocked` / `force_unlocked` / `not_locked` / `others_lock` / `skipped_pair` / `failed`、`ok`、他人のロックなら `owner` と `locked_at`）を出力します。

## ロックが必要なファイル（lockable）と、コミット前の確認

Perforce の「チェックアウトしないと編集できない」に近い仕組みを、Git LFS の `lockable` で作ります。すべてのファイルではなく、指定したファイルだけが対象です。

### 対象のファイルを決める

リポジトリの `.gitattributes` に `lockable` を付けます。LFS で管理していないファイル（テキストの `.unity` や `.prefab`）にも付けられます。

```text
*.unity   lockable
*.prefab  lockable
Assets/Art/**/*.psd  filter=lfs diff=lfs merge=lfs -text lockable
```

- `.gitattributes` もコミットして、チームで同じものを使います。
- Unity の `.meta` も守りたいときは、`*.unity.meta lockable` のように別に書きます（`repo-tweaks lock` は `.meta` も一緒にロックします）。

### 3 段階で守る

| いつ | 仕組み | 内容 |
|---|---|---|
| 編集するとき | Git LFS（もとからある） | 自分がロックしていない対象ファイルは、読み取り専用になる。`repo-tweaks lock` で書き込めるようになり、解除すると読み取り専用に戻る |
| コミットするとき | pre-commit（`repo-tweaks check`） | 対象ファイルを、自分のロック無しで変更・削除していたらコミットを止める |
| push するとき | Git LFS（もとからある） | 他人がロック中のファイルを変更していたら push を止める。**ただし LFS で管理しているファイルだけ**（Git LFS は、送る LFS のデータについてロックを確かめるため）。LFS 外の `.unity` などは、コミットの段でだけ止まる |

- 読み取り専用は、checkout・commit・merge のあとに Git LFS が付け直します（`git lfs install` の hook が必要）。
- **新しく追加したファイル**は、ロック無しでコミットできます（まだ誰も触っていないため）。ただし、そのパスを他人が先にロックしていれば止めます。リネームは「元のファイルの削除 + 新しいファイルの追加」として扱うので、元のファイルのロックが必要です。
- **マージのコミット**では、相手のブランチから来た変更はそのまま通します。マージの途中で自分が手を入れたファイル（両方の親と違うもの）だけ、ロックが必要です。
- **GitHub につながらない**など、ロックを確かめられないときは、警告を出してコミットを通します（オフラインでも作業を止めないため）。
- どうしても止められたくないときは `git commit --no-verify` で飛ばせます（GitHub はサーバー側でロックを強制できないため、最後の砦ではありません）。

止められたときの表示：

```text
repo-tweaks: ロックしていないファイルを変更しているため、コミットを止めました
  Assets/Scenes/Main.unity  ロックなし
  Assets/Art/hero.psd       bob がロック中
ロックしてから、もう一度コミットしてください:
  repo-tweaks lock Assets/Scenes/Main.unity
他人がロック中のファイルは、変更を取り消すか、ロックしている人と相談してください
```

### 状態を見る（status）

```text
$ repo-tweaks status
ロックが必要なファイル 120 件のうち、ロック中・変更中のもの（3）
  Assets/Art/hero.psd          bob がロック中
  Assets/Scenes/Main.unity     自分がロック中  変更あり
  Assets/Prefabs/Enemy.prefab  ロックなし      変更あり  ⚠ ロックが必要

⚠ ロックせずに変更しているものが 1 件あります。repo-tweaks lock でロックしてからコミットしてください
```

- 既定では、ロック中か変更中のものだけを表示します。`--all` ですべて表示します。
- `--json` は、すべての対象ファイルについて `path`、`lock`（`none` / `ours` / `theirs`）、`owner`、`locked_at`（UTC）、`change`（`null` / `modified` / `deleted` / `added`）、`problem`（`null` / `not_locked` / `held_by_other`。このままではコミットできないもの）を出力します。

## Discord への通知

`repo-tweaks lock` / `unlock` が成功すると、CLI がサーバーへ送り、サーバーが Discord に投稿します（1回の操作で1メッセージ。`.meta` は、元のファイルと持ち主が同じなら、その行にまとめます）。

```text
🔒 alice がロックしました（example-org/game）
・ Assets/Scenes/Main.unity（.meta も）
・ Assets/Art/hero.psd

🔓 alice が解除しました（example-org/game）
・ Assets/Scenes/Main.unity（.meta も）

⚠️ alice が bob さんのロックを強制解除しました（example-org/game）
・ Assets/Art/hero.psd（10/14 10:00 から）
```

- 名前は GitHub / GitLab のユーザー名（`list` の持ち主と同じ）です。ロックした直後は GitHub が名前を返さないので、ロックの一覧から自分の名前を引き直します。それでも分からないときは git の `user.name` を使います。リポジトリは origin の URL（`github.com/` などは省きます）です。
- ファイルが多い・パスが長いときは、Discord の文字数に収まる分だけ並べ、残りは「ほか N 件」にします。長いパスは先頭を「…」で省きます（ファイル名が見えるように）。
- サーバーが設定されていない（`repo-tweaks.server` が無い）ときは送らず、「送り先が未設定なので送っていません」と1行出します（設定し忘れに気づけるように。通知を使わないなら `repo-tweaks.notify false` で、この表示も消えます）。サーバーはあるのにトークンが無いとき・サーバーに届かないときは警告を出します（最初に失敗した時点で、残りの通知は送りません）。ロック・解除は済んでいるので、終了コードは変えません。
- 管理画面で Webhook URL を登録していないときは、サーバーは Discord に送りません（ログにも警告を出しません）。
- 送りたくないときは `--no-notify` を付けるか、git config で止めます。

  ```bash
  git config --global repo-tweaks.notify false
  ```

- Fork や `git lfs lock` で直接取ったロック・解除は、CLI を通らないのでここでは通知されません。下の「ロックの照合」で拾います。
- ファイル名や名前に `@everyone` などがあっても、Discord のメンションにはなりません（送信時にメンションを止めています）。
- サーバーは受け取った応答を先に返し、Discord への投稿はそのあとで行います（CLI の 3 秒の打ち切りに影響しないため）。投稿に失敗したときは、サーバーのログに残ります。

## ロックの照合と滞留の警告（サーバー）

サーバーが一定の間隔で、管理画面に登録したリポジトリのロック一覧を GitHub から取り、前回と比べます。

```text
🔍 ロックを見つけました（example-org/game）
・ Assets/Scenes/Main.unity（.meta も、bob さん、10/17 09:00 から）

🔓 ロックが外れました（example-org/game）
・ Assets/Art/hero.psd（bob さんのロック、10/14 10:00 から）

⏰ 3日以上ロックされたままのものが 1 件あります
・ Assets/Art/hero.psd（example-org/game、bob さん、10/14 10:00 から、4日1時間）
```

- **見つけました / 外れました**：CLI を通らずに取られた・外れたロック（Fork や `git lfs lock` など）だけを流します。CLI がすでに通知したものは、ロックの ID で見分けて流しません。
  - 取られてから 2 分以内のロックは、CLI からの通知がまだ届いていないだけかもしれないので、「確認待ち」にして、約 2 分後にもう一度照合してから判断します（いつもの間隔を待ちません）。
  - リポジトリは大文字・小文字を区別せずに突き合わせます（登録した URL と、各自の origin の表記が違っても同じものとして扱います）。
- **登録した直後の 1 回目**は、今あるロックを数えるだけで、通知しません。
- **照合に失敗し始めたとき**（トークンの期限切れなど）と、**直ったとき**に、1 回ずつ通知します。失敗の理由は管理画面にも出ます。1 つのリポジトリで失敗しても、ほかのリポジトリの照合は続けます。
- **滞留の警告**は、毎日決まった時刻に 1 回、照合で取ったロック一覧から作ります（GitHub への追加の問い合わせはしません）。照合に失敗しているリポジトリは、一覧が古いかもしれないので含めず、そのことを書き添えます。
- 状態（前回のロック一覧・通知済みの ID）は、ボリュームの `state.json` に保存します。再起動しても続きから比べます。
- 今は **GitHub（github.com）だけ**に対応しています。登録された URL が github.com 以外なら断り、認証情報を送る先も github.com に限っています（リダイレクトも追いません）。

### リポジトリの登録に使うトークン

管理画面の「ロックの照合」で、リポジトリの URL、GitHub のユーザー名、トークンを登録します。トークンはロックの一覧を**読むだけ**に使います。

- **Fine-grained トークン**をおすすめします。GitHub の「Settings → Developer settings → Personal access tokens → Fine-grained tokens → Generate new token」で、次のように作ります。
  - Repository access：**Only select repositories** で、照合するリポジトリだけを選ぶ
  - Permissions → Repository permissions：**Contents を Read-only**（Metadata の Read-only は自動で付きます）。これで足ります（実機で確認済み）
  - Expiration（期限）は好みで。期限が切れると照合が失敗し、Discord に 1 回知らせが届くので、そのときに作り直して登録し直します
- classic トークンでも動きますが、スコープ（`repo` など）がアカウントのすべてのリポジトリにかかるので、おすすめしません。
- リポジトリが組織（Organization）のものなら、Resource owner にその組織を選びます（組織の設定で Fine-grained トークンが許可されている必要があります）。
- 登録したら「今すぐ照合」を押し、「状態」が `OK` になるか確かめてください。
- トークンはサーバーの設定ファイル（ボリュームの `admin.json`）に保存し、画面には最後の 4 文字だけを出します。登録を削除すると、トークンも消えます。

## サーバーの API

| メソッド・パス | 認証 | 内容 |
|---|---|---|
| `GET /health` | なし | 動いているかの確認 |
| `GET /api/ping` | Bearer | トークンを含めた疎通確認 |
| `POST /api/events` | Bearer | 通知を受け取る（本文は 64KB まで。`lock` / `unlock` / `force_unlock` は `files` が必須で、Discord にも投稿する。`files` の `id` は照合での二重の通知を防ぐのに使う） |
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
