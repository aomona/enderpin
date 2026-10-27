# Enderpin

Minecraftのクライアントとサーバーを準備・共有するCLI。Minecraft・Java・Fabric・Paper・Velocity・MODのバージョンを固定し、別のPCでも同じ構成を再現できます。

[ドキュメント](https://enderpin-docs.aomona.workers.dev/) · [対応環境](https://enderpin-docs.aomona.workers.dev/platforms)

## 名前付きターゲット

一つの`enderpin.toml`と`enderpin.lock`でクライアント・複数サーバー・プロキシを管理できます。構成形式は4です。設定ファイルは`survival/files/`、実行環境・ワールドは`survival/run/`のように対象ごとにまとめます。

```sh
# enderpin.tomlを記述してから、全ターゲットと標準設定を準備
enderpin init --target all
enderpin run --target survival
```

[設定と継承ルール](docs/configuration.md) · [初期設定の生成](docs/setup.md) · [配置と同期の仕組み](docs/workspace.md) · [旧形式からの移行](docs/migration.md)

**v0.2.0はformat 4です。v0.1.8以前の環境は[移行手順](docs/migration.md)を確認してください。既存ワールドは自動移動しません。**

## 公開版ではじめる

Node.js 18以上とnpm、またはBunを使います。

```sh
npm install -g https://github.com/aomona/enderpin/releases/download/v0.2.0/enderpin-0.2.0.tgz
# または: bun install -g https://github.com/aomona/enderpin/releases/download/v0.2.0/enderpin-0.2.0.tgz

enderpin init my-world --loader fabric
cd my-world
enderpin add sodium
enderpin login  # 初回のアカウント認証
enderpin run
```

`init` で版・ローダー・MODを選び（MODを使う場合はFabric）、`run` で権限を確認して起動します。アカウント認証は既定で有効です。

- [ログイン・サーバー起動](https://enderpin-docs.aomona.workers.dev/launch)
- [MODの追加・更新](https://enderpin-docs.aomona.workers.dev/packages)
- [設定を共有する](https://enderpin-docs.aomona.workers.dev/sharing)
- [Velocity・複数サーバー・Ansible CD](docs/deployment.md)
- [開発・ビルド手順](https://enderpin-docs.aomona.workers.dev/contributing)

> v0.2.0はGitHub Releaseから取得できます。npmレジストリのv0.1.3には新CLIと `files/`・`run/` の新構成が含まれないため、上のURLを指定してください。旧版から更新する場合は[移行手順](docs/migration.md)を確認してください。

v0.1.5以降では `enderpin wrapper` でリポジトリ用Wrapperを生成できます。3ファイルを構成リポジトリへコミットすれば、共有先は `./enderpinw sync --locked` から使えます。Enderpin・Node.js・Bunの事前インストールは不要です。[Wrapperの使い方](docs/installation.md#repository-wrapper)を参照してください。

## ライセンス

[GPL-3.0-only](LICENSE) · [第三者のコード・ライセンス](https://enderpin-docs.aomona.workers.dev/third-party)
