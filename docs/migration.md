# 旧形式から移行する

自動移動・旧形式との互換対応は行いません。ゲームを終了し、旧ワークスペース全体をバックアップしてから、別の空ディレクトリで移行します。

1. 旧環境の `enderpin.toml` と `enderpin.lock` を新しいディレクトリへコピーし、**両ファイルのトップレベルの** `format = 2` を `format = 3` に変更します。旧 `.enderpin/` とゲームディレクトリはコピーしません。
2. 新しいディレクトリに `files/common/`・`files/client/`・`files/server/` を作り、共有したい設定だけを旧 `client/`・`server/` から対応する `files/` へコピーします。
3. `.gitignore` に `/run/`・`/.enderpin/`・`/.enderpinignore` を追加します。`enderpin sync --target all` を実行して、新形式の指紋と配置を生成します。この最初の同期は `--locked`・`--offline` を付けず、`update` は使いません。既存ロックの固定版を維持して解決するため、変更されたロックを確認してください。
4. 旧 `client/saves/`・`server/world/` などの必要なプレイデータを、それぞれ新 `run/client/`・`run/server/` へコピーします。個人設定も必要に応じてコピーしますが、手順2で共有したファイルと生成済みの管理MODは上書きしません。サーバーの `level-name` を変えている場合は、そのワールドもコピーします。
5. `enderpin prepare --target all` で実行環境を準備します。既存の実行環境の固定版は維持し、旧ロックで未準備だった対象はここで固定します。必要な対象が片側だけなら `--target client` または `--target server` を指定します。その後、必要な権限・外部フォルダを設定して再承認します。起動してワールドと設定を確認できるまで旧環境を残してください。新しい構成・ロック・`files/` をGitへ追加します。

## 権限設定の新形式へ移行する

MXCへの移行準備として、権限の共有設定を `permissions` にまとめました。
この段階では起動エンジンは既存のSeatbelt / bubblewrap / AppContainerです。MXCへの切り替えはまだ完了していません。
ワークスペース・ロックの `format = 3` と `files/`・`run/` の配置は変えません。

ゲームを終了し、設定をバックアップしてから、`enderpin.toml` を次のように変更します。自動変換は行いません。

| 旧指定 | 新指定 |
| --- | --- |
| `[client.sandbox]` / `[server.sandbox]` | `[client.permissions]` / `[server.permissions]` |
| `game_write = true` / `false` | `[<side>.permissions.files]` の `game = "read-write"` / `"read-only"` |
| `read_only = [...]` | `[<side>.permissions.files]` の `read_only = [...]` |
| `clipboard = true` / `false` | `clipboard = "read-write"` / `"none"`（Windowsの個別指定、Linuxの `"none"` は拒否） |
| `clipboard` を省略 | 省略のまま、または `clipboard = "system"` |
| その他のキー | 名前と値を変えずに `permissions` へ移す |

ファイルのキー以外を `permissions.files` 内に入れないようにします。未知のキーや旧 `sandbox` テーブルは拒否し、既存の禁止を黙って標準値へ戻しません。
Windowsではクリップボードの個別指定をまだ強制できないため、省略または `"system"` のみ受け付けます。
Linuxでもdisplayへのアクセスとクリップボードの禁止を分離できないため、`"none"` は拒否します。
マイクを省略した場合のOS依存の動作も変更していません。

外部フォルダとプレイデータを削除する必要はありません。以前の承認は無効になるため、
`enderpin permissions show` で計画を確認し、`enderpin permissions approve` で再承認します。
