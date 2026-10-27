# enderpin.toml の設定

`format = 4` は一つのmanifestとlockで、複数のクライアント・ゲームサーバー・プロキシを管理します。v0.2.0以降に対応する形式です。旧版の環境は[移行手順](migration.md)を確認してください。

```toml
format = 4
minecraft = "26.3"

[groups.common.packages]
lithium = { modrinth = "lithium", version = "xS0Q8LSi" }

[profiles.backend]
type = "server"
loader = "fabric"
loader_version = "0.19.5"
groups = ["common"]
jvm_args = ["-XX:+UseZGC"]
memory_mib = 4096

[targets.client]
type = "client"
loader = "fabric"
loader_version = "0.19.5"
groups = ["common"]

[targets.survival]
profile = "backend"

[targets.creative]
profile = "backend"
memory_mib = 3072
jvm_args = []

[targets.proxy]
type = "proxy"
loader = "velocity"
loader_version = "4.2.0+30"
memory_mib = 512
```

版は記法の例で、最新版を意味しません。`type`は`client`・`server`・`proxy`。`proxy`はVelocity用です。`--target survival`は名前を選び、`--target all`は宣言された全ターゲットを選びます。省略時は`client`を選択し、存在しなければエラーです。`run`と`permissions`は一つの名前だけを受け付けます。

## 設定の境界

Enderpinが扱うのは実行種別、Minecraft・ローダーの版、パッケージ、メモリ、JVM引数、共有元と権限です。`server.properties`、`velocity.toml`、MOD・プラグインの設定は各ターゲットの`files/`に元の形式で置きます。ポート・ゲームモード・接続先をmanifestへ重複して書きません。任意の設定本文の埋め込み、文字列展開、シェルコマンド、条件式はありません。

ルートの`minecraft`は既定値で、ターゲットまたはプロファイルの`minecraft`で置換できます。Velocityの配布版は`loader_version`で独立して選択します。現在のランタイム解決はプロキシでもMinecraftメタデータを保存し、プラグイン解決では指定Minecraft版で互換性を検証します。

## プロファイルとグループ

`profiles`は起動設定のひな形で、実行ディレクトリを持ちません。ターゲットが参照できるプロファイルは一つです。プロファイルから別のプロファイルを継承することはできません。

- 文字列・数値はターゲットの指定が優先します。
- 配列（`groups`・`file_groups`・`jvm_args`・`enabled`・`disabled`）は全体を置換します。省略は継承、`[]`は空への置換です。
- `packages`は名前単位で合成します。同名の宣言はターゲット側の項目全体で置換し、URLとModrinth指定を混ぜません。
- パッケージグループは`groups.<名前>.packages`だけを持ちます。グループ間の同名宣言は同一なら一つにまとめ、異なるなら記述順によらずエラーです。その後、プロファイル・ターゲットの直接宣言を適用します。
- 権限は`targets.<名前>.permissions`に直接指定します。プロファイルへの権限指定は拒否し、承認は各PC・各ターゲットに分離します。

グループのパッケージは配布元のクライアント／サーバー情報に従って選択します。直接の`packages`指定や`enabled`は環境情報の判定を上書きしますが、ローダー互換性と必須依存は常に検証します。`disabled`は導入を除外し、必須依存を欠く場合は停止します。

`add --target all`は各ターゲットへ直接追加します。共有宣言にしたい場合はmanifestの`groups`を編集します。`remove`は対象の直接宣言を削除し、共有・プロファイルから再導入されないよう対象の`disabled`に記録します。ほかの対象や共有宣言は変更しません。

## パッケージと起動条件 {#jvm-options}

Modrinthの`version`は版のIDで、semver範囲ではありません。省略すると互換版を解決してlockに固定します。URL指定では既存と同様に公開HTTPSと取得内容のハッシュ検証を使用します。Paper/Velocityのプラグインは`kind = "plugin"`、MODは`kind = "mod"`です。

`memory_mib`の既定値は2048、範囲は256〜1048576です。`run --memory`はその実行だけ上書きします。`jvm_args`は許可された`-XX:`と`-D`だけで、classpath・agent・認証などの予約オプションは拒否します。

名前は1〜64文字の小文字ASCII英数字・`-`・`_`です。パスやWindows予約名は使えず、ターゲットには`all`・`shared`・`run`・`files`・`enderpin`・`target`を使えません。

```toml
[targets.client.permissions]
network = true
clipboard = "system"

[targets.client.permissions.files]
game = "read-write"
read_only = ["mods", "resourcepacks"]
```

標準値は省略できます。外部フォルダと承認は`.enderpin/<名前>/permissions.toml`に保持します。ゲームに認証トークンや秘密鍵を渡さない既存の境界を維持します。[サンドボックスと権限](sandbox.md)を参照してください。
