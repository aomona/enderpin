# Enderpin

Minecraftのクライアントとサーバーを準備・共有するCLI。Minecraft・Java・Fabric・Paper・Velocity・MODのバージョンを固定し、別のPCでも同じ構成を再現できます。

[ドキュメント](https://enderpin-docs.aomona.workers.dev/) · [対応環境](https://enderpin-docs.aomona.workers.dev/platforms)

## はじめる

Node.js 18以上とnpm、またはBunを使います。

```sh
npm install -g https://github.com/aomona/enderpin/releases/download/v0.1.4/enderpin-0.1.4.tgz
# または: bun install -g https://github.com/aomona/enderpin/releases/download/v0.1.4/enderpin-0.1.4.tgz

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

> v0.1.4はGitHub Releaseから取得できます。npmレジストリのv0.1.3には新CLIと `files/`・`run/` の新構成が含まれないため、上のURLを指定してください。旧版から更新する場合は[移行手順](docs/migration.md)を確認してください。

## ライセンス

[GPL-3.0-only](LICENSE) · [第三者のコード・ライセンス](https://enderpin-docs.aomona.workers.dev/third-party)
