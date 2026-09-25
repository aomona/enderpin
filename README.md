# Enderpin

**Minecraftのクライアントとサーバーを、同じ設定で準備・共有するCLI。**

Minecraft・Java・Fabric・MODのバージョンを固定し、別のPCでも同じ構成を再現できます。ゲームはOSのサンドボックス内で起動し、通信・ファイル書き込み・アカウント認証の許可を確認できます。

[ドキュメント](https://enderpin-docs.aomona.workers.dev) · [はじめかた](docs/setup.md) · [コマンド一覧](docs/commands.md) · [対応環境](docs/platforms.md)

> このREADMEとドキュメントは **mainブランチ** の仕様です。`files/`・`run/` を使う新構成（`format = 3`）は、公開済み **v0.1.3にはまだ含まれません**。新構成を試す場合は [ソースからインストール](docs/installation.md#最新のソース版) してください。

## まず使ってみる

公開版のインストールには、Node.js 18以上とnpmを使います。Bunも利用できます。

```sh
npm install -g enderpin
# Bunの場合: bun install -g enderpin
enderpin --version
```

空のディレクトリで、Minecraftの版・ローダー・MODを選びます。

```sh
mkdir my-world
cd my-world
enderpin quick
enderpin launch
```

`quick` は必要なファイルを取得するセットアップです。ゲームは `launch` で起動し、初回は権限を確認します。最初のクライアントはオフラインの `Player` です。認証が必要なサーバーへの参加は [ログインと起動](docs/launch.md) を参照してください。

サーバーも準備する場合は、初回の `quick` に `--server` を付けます。起動時は [Minecraft EULA](https://aka.ms/MinecraftEULA) に同意した場合のみ、次を実行します。

```sh
enderpin quick --server
enderpin launch --target server --accept-eula
```

既存環境に `quick` を重ねて実行しても設定は上書きしません。[非対話セットアップ](docs/setup.md)・[インストール方法の詳細](docs/installation.md) も用意しています。

## MODを管理する

Fabricでセットアップした環境では、ModrinthのIDやslugで追加できます。

```sh
enderpin add lithium --target all    # 共通指定へ追加
enderpin add sodium --target client  # クライアントだけへ追加
enderpin list --target all           # 固定済みの内容を確認
enderpin update --target all         # 対応する新しい版へ更新
```

`--target` を省略するとクライアントが対象です。通常の `sync` は固定済みの版を維持し、`update` で明示的に更新します。[MOD・依存関係・個人用の除外設定](docs/packages.md) を参照してください。

## 何を共有して、何を残すか

mainブランチの新構成では、共有する入力とプレイデータを分けます。

```text
my-world/
├── enderpin.toml        # 希望する版・MOD・権限設定
├── enderpin.lock        # 解決済みの配布版とハッシュ
├── files/              # Gitで共有する設定・ファイル
│   ├── common/         # client / server 共通
│   ├── client/         # client 固有の上書き
│   └── server/         # server 固有の上書き
├── run/                # 実際のゲーム環境。Git管理外
│   ├── client/         # mods、config、saves など
│   └── server/         # mods、config、world など
└── .enderpin/          # 同期台帳・ローカル承認・復旧情報
```

**Gitへ入れるものは `enderpin.toml`・`enderpin.lock`・`files/`。`run/` はワールドを含むので、消してよいキャッシュではありません。**

`files/common/config/example.json` を置いて同期すると、両側へコピーされます。同名の対象別ファイルがあればそちらを優先します。ゲーム側だけの編集は保持し、共有元との競合は上書きせずに知らせます。

```sh
enderpin sync --target all --locked
```

詳しくは [共有設定の反映ルール](docs/workspace.md)、[別のPCで再現する手順](docs/sharing.md)、[旧形式からの移行](docs/migration.md) を参照してください。

## 起動する前に

- **権限**：`enderpin permissions` で編集します。初回・権限変更・MOD更新時は再承認します。[設定とOSごとの制限](docs/sandbox.md)
- **Linux**：bubblewrapとユーザー名前空間が必要です。[対応環境](docs/platforms.md)
- **セーブデータ**：Enderpinはワールドの自動バックアップを行いません。構成の再現とワールドの保護は別に管理してください。
- **検証範囲**：Javaの隔離テスト、CI、Minecraftの実起動は区別して記録しています。[検証記録](docs/verification.md)

## 開発する

Rust 1.89以上・JDK 17以上・Cコンパイラーが必要です。

```sh
cargo build --locked
cargo test --locked
```

[開発・配布・ドキュメントサイトの更新](docs/contributing.md) に検証コマンドをまとめています。Rustライブラリとしても利用できます。

## ライセンス

[GPL-3.0-only](LICENSE)。[MonaLauncherからの再利用箇所](docs/third-party.md) を含みます。
