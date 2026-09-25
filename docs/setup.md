# 初期セットアップ

v0.1.2からのセットアップ動作です（v0.1.1の `quick` はゲーム起動コマンド）。

```sh
enderpin quick           # クライアントをセットアップ
enderpin quick --server  # クライアントとサーバーの両方をセットアップ
```

1. Minecraftのバージョンを検索して選択します。文字入力で一覧を絞り込めます。
2. ローダーを検索して選択します。現在の候補はVanillaとFabricです。
3. Fabricの場合、ModrinthのMODをキーワード検索し、結果から追加・取り消しできます。追加済みは `[x]` と表示します。次のページ・再検索にも対応します。VanillaではMOD選択を省略します。
4. 構成を確認すると、Minecraft・Java・ローダー・MODと必須依存関係を取得して終了します。ゲームは起動しません。

MODの候補は選んだMinecraft版とローダーで絞り込み、選択時に対象側で利用できるリリースを確認します。`--server` では両側に対応するMODを対象ごとに配置します。任意依存は自動追加しません。古いMinecraftなど実行環境が未対応の場合は準備時に理由を示して停止します。

保存先は実行ディレクトリ内の `run/client/`・`run/server/` と、共通の `enderpin.toml`・`enderpin.lock` です。`-C DIRECTORY` で保存先を変更できます。初回に選んだ版を固定し、既存の設定は `quick` で上書きしません。MODの追加は `add`、準備の再実行は `prepare` を使います。別の版は別ディレクトリでセットアップしてください。

選択を省略したり、非対話環境でセットアップする場合は引数で指定できます。

```sh
enderpin quick --no-interactive --version 1.21.1 --loader fabric --mods lithium,sodium
enderpin -C my-server quick --server --no-interactive --version 26.2 --loader vanilla
```

`--version` と `--loader` は対応する選択画面を省略します。`--mods` はModrinthのIDまたはslugをカンマ区切りで指定します。非対話環境では版とローダーの指定が必須です。

セットアップ後に起動するには、保存先で次を実行します。

```sh
enderpin launch
# Minecraft EULAを確認し、同意した場合
enderpin launch --target server --accept-eula --port 25566
# バニラの起動でサンドボックスを無効にする場合
enderpin launch --no-sandbox
```

サンドボックス権限は起動時に確認します。セットアップではMODの権限を自動承認しません。クライアントの初期設定はオフラインPlayerで、認証必須サーバーには参加できません。サーバーはMinecraft標準の認証設定を使い、EULA同意は起動時に行います。`--port`・`--memory`・`--no-sandbox`・`--accept-eula` は `launch` に指定してください。


次は [ファイル構成と共有設定](workspace.md) または [ログインと起動](launch.md) へ進みます。
