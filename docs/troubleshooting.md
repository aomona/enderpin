# 困ったときは

まず `enderpin --version` と、実行しているディレクトリを確認してください。このサイトはmainブランチの `format = 3` を説明しています。

## `enderpin` が見つからない

インストールに使ったnpmまたはBunのグローバル実行ファイルの場所をPATHへ追加します。Bunは `bun pm bin -g` で確認できます。[インストール方法](installation.md) を参照してください。

## `quick` で既存設定を変えられない

`quick` は初回用で、既存の `enderpin.toml` と `enderpin.lock` を上書きしません。MODの追加は `add`、取得のやり直しは `prepare`、別のMinecraft版は別ディレクトリでセットアップします。

## `configuration differs from the lock`

構成とロックが一致していません。意図して `enderpin.toml` を変更したなら、ゲームを終了し、対象を指定して `sync` を実行してからロックの差分を確認します。

```sh
enderpin sync --target all
```

Gitから受け取った構成なら、共有元でロックもコミットされているかを確認してください。

## `shared file conflict`

共有元とゲーム側の両方が変更されています。ゲーム側の内容を残すなら、対応する `files/` へコピーして再同期します。共有元へ戻すなら、対象のローカル変更を破棄することを確認して `restore` を使います。

```sh
enderpin restore --target client
```

`restore` は対象の管理MODと共有ファイル全体に作用します。ワールドを含む `run/` 全体を削除して解決しないでください。

## `unmanaged filename collision`

Enderpinが管理していない同名ファイルがあります。必要な内容をバックアップし、同名ファイルを別の場所へ退避してから同期します。`restore` でも管理外ファイルを上書きしません。

## `managed file was modified`

固定されたMODなどのファイルが変更されています。共有元のMOD設定を編集したい場合は、JARではなく `files/` 内の設定を変更します。固定済みの配布ファイルへ戻す場合のみ `restore` を使います。

## オンラインサーバーに参加できない

`quick` の初期設定はアカウント認証が無効です。`enderpin permissions` でクライアントのアカウント認証を有効にして保存し、`enderpin login` を実行します。[ログインと起動](launch.md) の順序を確認してください。

## 権限の承認を求められる

初回や、権限・MOD・共有管理下のファイルが変わった場合に承認が必要です。`enderpin permissions show` で内容を確認し、意図した変更なら承認してください。ゲームを実行中の場合は先に終了します。

## サンドボックスを開始できない

Linuxではbubblewrapと利用可能なユーザー名前空間が必要です。ほかにもOSごとの制限があります。[対応環境](platforms.md) と [権限の制約](sandbox.md) を確認してください。失敗時に自動で隔離なし起動へ切り替えることはありません。

## `unsupported manifest format`

実行ファイルとワークスペースの形式が異なります。v0.1.3は `format = 2`、このサイトのmainは `format = 3` です。[インストール](installation.md) と [手動移行手順](migration.md) を確認してください。既存環境を直接書き換える前に、全体をバックアップします。
