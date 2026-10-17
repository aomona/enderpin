# enderpin.toml の設定

```toml
format = 3
minecraft = "1.21.1"

[common.lithium]
modrinth = "lithium"
kind = "mod"

[client]
loader = "fabric"
# 配布元の対象側の指定を上書きする場合に利用します。
enabled = []
disabled = []

[client.packages.sodium]
modrinth = "sodium"
kind = "mod"

[server]
loader = "fabric"
# Minecraft版と独立したFabric Loader版。省略時は安定版を解決します。
loader_version = "0.16.10"
```

構成内のModrinth識別子はIDまたはslugです。CLIから追加すると安定したプロジェクトIDを保存します。特定の配布版を固定するには `version` にModrinthのバージョンIDを指定します。semver範囲は扱いません。

共通指定は両側へ展開し、配布元の環境情報から初期対象を選びます。対象側の `packages` に明示した項目、または `enabled` で指定した項目は対象側のメタデータ判定を上書きします。`disabled` は導入しません。実際のローダー互換性・必須依存の検証は引き続き行います。

同じ名前の対象別パッケージ指定は、共通指定の項目全体を置き換えます。変更後は対象ごとにロックを更新してください。片側だけの操作で反対側のファイルを取得・変更しません。共通指定を手編集した後は `enderpin lock --target all` で両側の解決結果だけを保存できます。

## ゲーム全体の権限

権限はMODごとではなく、Minecraftのプロセス全体に適用します。
標準値は省略でき、`enderpin permissions` の編集画面も変更した項目だけを保存します。

```toml
[client.permissions]
network = true
audio = true
clipboard = "system"

[client.permissions.files]
game = "read-write"
read_only = ["mods", "resourcepacks"]
```

Java・検証済みランタイム・一時領域のパスはEnderpinが準備します。設定への列挙は不要です。
外部フォルダの絶対パスと承認は各PCの `.enderpin/<client|server>/permissions.toml` に保存し、共有設定には含めません。
OSごとの強制可能範囲と認証の扱いは [サンドボックスと権限](sandbox.md) を参照してください。
旧 `[client.sandbox]` / `[server.sandbox]` はエラーになります。移行方法は [移行手順](migration.md#権限設定の新形式へ移行する) を参照してください。

サーバーの`loader`には`paper`と`velocity`も指定できます。Paperは`loader_version`にビルド番号、Velocityは`版`または`版+ビルド番号`を指定します。プラグインは`kind = "plugin"`です。[複数サーバーとAnsible CD](deployment.md)を参照してください。
