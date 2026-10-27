# コマンド

普段は `init` → `add` → `run` を使います。構成を編集したり別のPCへコピーした場合は `sync`、固定済みの版を更新する場合は `update` です。

```sh
enderpin init my-world --loader fabric
cd my-world
enderpin add sodium
enderpin login
enderpin run
```

## 初期設定と日常操作

| コマンド | 動作 |
| --- | --- |
| `init [DIRECTORY]` | 対話式にワークスペースを作り、Minecraft・Java・パッケージを取得。標準設定を生成。通常のゲームセッションは起動しない |
| `add ID_OR_URL` | パッケージを追加して同期 |
| `remove NAME` | 直接指定したパッケージを削除して同期。ほかで必要な依存は維持 |
| `search [QUERY]` | Modrinthを検索。端末では選択して追加、非対話環境では一覧を表示 |
| `list` | 固定されたパッケージと、パッケージ構成に対するロックの更新要否を表示 |
| `sync` | Minecraft・ローダー・Java・パッケージ・共有ファイルを準備。変更のない構成では固定済みの版を維持 |
| `run` | 固定された一つのターゲットを起動。不足ファイルはロックに従って取得し、版は更新しない |
| `update` | ランタイムとModrinthパッケージの版を更新。明示的なローダー版・パッケージ版・URLの固定は維持 |

`init` のMinecraft版は `--minecraft VERSION`、ローダーは `--loader vanilla|fabric|paper|velocity`、ローダーの固定版は `--loader-version VERSION` で指定します。名前付き環境は`--target survival --type server`などで作成します。Velocityは`--type proxy`です。詳しくは [初期セットアップ](setup.md) を参照してください。

`run --memory MIB` はJavaの最大ヒープで、省略時は対象の`memory_mib`（未指定なら2048 MiB）です。`--jvm-arg ARG` はこの起動だけに `-XX:` オプション・`-D` プロパティを追加します（繰り返し指定可）。常に使う場合は [`jvm_args`](configuration.md#jvm-options) に記載します。サーバーのEULA同意は `--accept-eula`、標準入力なしの常駐起動は `--service` です。`--offline` はEnderpinのダウンロードをオフラインにし、`--no-network` はゲームのネットワークを禁止します。[ログインと起動](launch.md) を参照してください。

## 権限と認証

| コマンド | 動作 |
| --- | --- |
| `permissions` | 対話式にゲーム権限を編集 |
| `permissions show` | 権限・承認状態を表示。`--json` では完全なplanとfingerprintを取得 |
| `permissions approve [FINGERPRINT]` | 端末で確認して承認、または指定したfingerprintと一致する場合だけ承認 |
| `permissions revoke` | 保存した承認を取り消す |
| `permissions allow-folder PATH [--write]` | 既存の外部フォルダを許可。既定は読み取り専用。承認は解除 |
| `permissions remove-folder PATH` | 外部フォルダの許可と承認を解除。フォルダ自体は保持 |
| `login [--client-id ID]` / `logout` | Microsoftアカウントでログイン／Enderpinの保存資格情報を削除 |

`--yes` はゲーム権限を承認しません。自動化でも、現在のplanを確認したうえで正確なfingerprintを渡す必要があります。

## 自動化と復旧

| コマンド | 動作 |
| --- | --- |
| `wrapper [--version VERSION] [--force]` | リポジトリ用の起動スクリプトと、Enderpinの固定版・SHA-256を生成。既存ファイルの置き換えは `--force` が必要 |
| `sync --locked` | パッケージとランタイムのロックが現在の構成と一致する場合だけ準備 |
| `sync --locked --offline` | 一致するロックとローカルキャッシュだけを使って準備 |
| `lock` | パッケージの解決結果を保存。ゲームファイルの配置・ランタイムの準備はしない |
| `restore` | ローカルの管理ファイル変更を破棄し、パッケージはロック、共有ファイルは現在の `files/` の内容へ復元 |

`lock`・`restore` でも `--offline` を使えます。`sync`・`restore` の `--no-ignore` はローカルの `.enderpinignore` による除外を無視します。`update` はネットワークから版を選び直す操作です。

## オプションの置き場所

`-C DIRECTORY` と `--cache-dir DIRECTORY` はコマンドの前後どちらにも書けます。それ以外は対象コマンドの後に指定します。

```sh
enderpin -C my-world sync --target all --locked
enderpin permissions show --target server --json
enderpin run --target server --accept-eula --memory 1024
enderpin run --target server --jvm-arg -XX:+UseZGC
```

- `--target` の既定は `client`。`init`・パッケージ操作・`sync`・`update`・`lock`・`restore`・`list` は `all` を指定でき、`run`・`search`・`permissions` は 宣言された一つのターゲット名を指定します。
- `--json` はパッケージ操作・`sync`・`update`・`lock`・`restore`・`list`・`permissions` で利用できます。`init`・`run`・認証操作にはありません。
- `--no-interactive` は `init`・`add`・`search`・`run`・`permissions`、`--yes` は `init`・`add`・`search` で利用できます。それ以外は対話操作を行いません。
- `enderpin --version` はEnderpin自体の版の表示です。`enderpin wrapper --version VERSION` はWrapperで固定するEnderpin版の指定です。Minecraft版の指定には使いません。

`enderpin --help` は用途別の一覧、`enderpin <COMMAND> --help` はそのコマンドの引数と使用例を表示します。旧CLIからの変更は [移行手順](migration.md#cli-changes) を参照してください。
