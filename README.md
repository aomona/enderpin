# Enderpin

Minecraftのクライアントとサーバーを準備・共有するCLI。Minecraft・Java・Fabric・Paper・Velocity・MODのバージョンを固定し、別のPCでも同じ構成を再現できます。

[ドキュメント](https://enderpin-docs.aomona.workers.dev/) · [対応環境](https://enderpin-docs.aomona.workers.dev/platforms)

## はじめる

Node.js 18以上とnpm、またはBunを使います。

```sh
npm install -g enderpin
# または: bun install -g enderpin

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

> ドキュメントはmainブランチの仕様です。新CLIと `files/`・`run/` の新構成は公開版v0.1.3には含まれません。利用する場合は[ソース版](https://enderpin-docs.aomona.workers.dev/installation#最新のソース版)をインストールしてください。

## ライセンス

[GPL-3.0-only](LICENSE) · [第三者のコード・ライセンス](https://enderpin-docs.aomona.workers.dev/third-party)
