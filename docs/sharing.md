# 別のPCと共有する

同じMODと設定を再現するには、構成・ロック・共有ファイルをGitへ保存します。ワールドや個人の認証情報は共有しません。

## 共有元で準備する

対象のゲームを終了してから実行します。

```sh
enderpin sync --target all
```

共有したいMOD設定などを `files/common/`、`files/client/`、`files/server/` に置き、反映を確認します。

```sh
enderpin sync --target all --locked
```

Gitへ追加するのは次のファイルです。v0.1.5以降の `init` はWrapperも生成します。既存の構成なら、v0.1.5以降で `enderpin wrapper --version 0.1.7` を一度実行してください。[生成手順と対応環境](installation.md#repository-wrapper)を参照してください。

```sh
git add enderpin.toml enderpin.lock files/ .gitignore
git add enderpinw enderpinw.ps1 enderpin-wrapper.properties
git commit -m "Pin Minecraft workspace"
```

`files/` が空ならGitは保存しません。その場合は `git add` から `files/` を省略できます。`run/`、`.enderpin/`、`.enderpinignore` はGit管理外にします。共有先は自分で選んだリポジトリへpushしてください。

## 別のPCで再現する

リポジトリをcloneし、そのディレクトリで必要な側だけ準備します。WrapperがあればEnderpinの事前インストールは不要です。Windowsでは `./enderpinw` を `.\enderpinw.ps1` に読み替えてください。

```sh
# クライアントを使うPC
./enderpinw sync --target client --locked
./enderpinw run --target client
```

```sh
# サーバーを動かすPC
./enderpinw sync --target server --locked
./enderpinw run --target server --accept-eula
```

サーバーのコマンドは [Minecraft EULA](https://aka.ms/MinecraftEULA) に同意した場合のみ実行してください。権限の承認・外部フォルダ・アカウントのログインはPCごとに設定します。共有元の承認は引き継ぎません。

## 変更を取り込む

ゲームを終了してGitの変更を取り込み、同じ対象を `sync --locked` で準備します。共有元とローカルの設定が食い違う場合は停止するので、[共有設定の競合](workspace.md#editing-settings) を解消してください。

`--locked` は構成とロックの不一致を拒否します。共有元でロックを更新し忘れた場合は、共有元で同期してロックもコミットします。受け取った側が勝手に最新版を選ぶためのオプションではありません。

## オフラインで準備する

必要なファイルがすべてキャッシュにある場合だけ使えます。

```sh
./enderpinw sync --target client --locked --offline
```

`--offline` はファイル準備に関する指定です。Wrapper自体も初回取得を済ませておく必要があります。認証付きクライアントのセッション更新や承認済みの認証仲介には、別途ホスト側のネットワークを使います。

## ワールドは別にバックアップする

構成のGit履歴から、ワールドの進行状況は復元できません。ゲームを正常終了してから `run/client/saves/` や `run/server/world/` をバックアップしてください。サーバーの `level-name` を変えた場合は、そのワールドディレクトリも対象です。
