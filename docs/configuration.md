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
```

構成内のModrinth識別子はIDまたはslugです。CLIから追加すると安定したプロジェクトIDを保存します。特定の配布版を固定するには `version` にModrinthのバージョンIDを指定します。semver範囲は扱いません。

共通指定は両側へ展開し、配布元の環境情報から初期対象を選びます。対象側の `packages` に明示した項目、または `enabled` で指定した項目は対象側のメタデータ判定を上書きします。`disabled` は導入しません。実際のローダー互換性・必須依存の検証は引き続き行います。

同じ名前の対象別パッケージ指定は、共通指定の項目全体を置き換えます。変更後は対象ごとにロックを更新してください。片側だけの操作で反対側のファイルを取得・変更しません。共通指定を手編集した後は `enderpin lock --target all` で両側の解決結果だけを保存できます。
