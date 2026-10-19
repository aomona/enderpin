# 初期セットアップ

`init` は版・ローダー・MODを選び、実行に必要なファイルを取得します。セットアップではゲームを起動せず、権限も自動承認しません。

```sh
enderpin init my-world
cd my-world
```

1. Minecraftのバージョンを検索して選択します。文字入力で一覧を絞り込めます。
2. ローダーを選択します。クライアントと両側はVanilla / Fabric、サーバーだけならPaper / Velocityも選べます。Velocityでは使用するVelocity版も入力します。
3. FabricではModrinthのMODを検索し、追加・取り消しできます。追加済みは `[x]` と表示し、ページ移動・再検索にも対応します。Vanilla・Paper・Velocityではこの選択を省略します。
4. 構成を確認すると、リポジトリ用Wrapperを生成し、Minecraft・Java・ローダー・MODと必須依存関係を取得して終了します。

v0.1.5以降の `init` は `enderpinw`・`enderpinw.ps1`・`enderpin-wrapper.properties` も作成します。生成時の最新の安定版EnderpinとSHA-256を固定するため、GitHubへの接続が必要です。既存のWrapperは上書きしません。[Wrapperの詳細](installation.md#repository-wrapper)を参照してください。公開済みv0.1.4の `init` にはこの生成処理はありません。

MODを使う場合はFabricを選んでください。候補は選んだMinecraft版とローダーで絞り込み、対象側で利用できるリリースを確認します。任意依存は自動追加しません。

ディレクトリを省略すると現在地に作成します。`enderpin -C my-world init` でも保存先を指定できますが、位置引数のディレクトリと `-C` の併用はできません。既存の構成・ロックを `init` で上書きしません。パッケージの追加は `add`、準備の再実行は `sync` を使います。

## 対象・バージョンを指定する

```sh
# Minecraft版だけ対話で選び、Fabricを使う
enderpin init my-world --loader fabric

# クライアントとサーバーの両側を非対話で準備
enderpin init my-network --target all --minecraft 1.21.1 \
  --loader fabric --loader-version 0.16.10 --mods lithium,sodium --no-interactive

# Paperサーバーだけを準備
enderpin init my-server --target server --minecraft 1.21.1 \
  --loader paper --loader-version 133 --no-interactive

# Velocityだけを準備（minecraftはパッケージの互換性判定に使う）
enderpin init my-proxy --target server --minecraft 1.21.1 \
  --loader velocity --loader-version 3.4.0-SNAPSHOT+563 --no-interactive
```

版は指定例です。`--target` の既定は `client`、`server` はサーバーだけ、`all` は両側です。`all` では選んだローダーを両側に設定し、MODは各側の対応状況に従って配置します。異なるローダーや版のサーバーを管理する場合は [複数ワークスペース](deployment.md) を使います。

`--minecraft` と `--loader` は対応する選択画面を省略します。両方を指定した場合は対話せず実行し、非対話環境では両方の指定が必須です。片方を対話で選ぶ場合の `--yes` は、任意のMOD選択と最後の確認を省略します。必要な版・ローダーの選択やゲーム権限の承認を省略する指定ではありません。

`--loader-version` はFabric Loader版・Paperビルド・Velocity版を固定します。Velocityでは必須で、ほかでは省略すると初回に解決してロックします。Vanillaには指定できません。`--mods` はFabric専用で、ModrinthのIDまたはslugをカンマ区切りで指定します。

共有設定は `enderpin.toml`・`enderpin.lock`・`files/`、ゲームの保存先は `run/client/`・`run/server/` です。指定した側だけを準備し、初回に選んだ版をロックします。古いMinecraftなど実行環境が未対応の場合は、準備時に理由を示して停止します。ダウンロード途中で失敗した場合は、保存された構成のディレクトリで `enderpin sync`（サーバーなら `--target server`、両側なら `--target all`）を実行してください。

## セットアップ後

```sh
# FabricクライアントへMODを追加
enderpin add sodium
enderpin login
enderpin run

# サーバーはEULAを確認し、同意した場合に起動
enderpin run --target server --accept-eula --port 25566
```

クライアントはアカウント認証が既定で有効です。初期設定はナレーター、クリップボード、リンク・フォルダ表示、ホストの確認付きファイル取り込みも許可します。`run` で現在の権限を確認して承認します。`--port`・`--memory`・`--no-sandbox`・`--accept-eula` は `run` に指定してください。サンドボックスを無効にできるのはVanillaだけです。

次は [ファイル構成と共有設定](workspace.md)、[ログインと起動](launch.md)、[サンドボックス](sandbox.md) を参照してください。
