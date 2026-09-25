# ログインと起動

`prepare` はMinecraft本体・Fabric・Temurin JDKの配布版とハッシュをロックし、現在のOS・CPU用のファイルを取得します。初回の両側準備後に、`enderpin.toml` と `enderpin.lock` をGitへ追加してください。別のPCでは必要な側だけを準備できます。

サーバーは [Minecraft EULA](https://aka.ms/MinecraftEULA) を確認し、同意する場合に明示して起動します。

```sh
enderpin launch --target server --accept-eula
```

同意はこのサーバーの `eula.txt` に保存します。ログを表示しながら `list`、`stop` 等のコンソール入力を使えます。Ctrl-Cは終了を要求し、2回目で強制停止します。保存完了まで自動的に待ち、時間だけを理由に強制終了しません。

## Microsoftアカウントで参加する

`quick` で作ったクライアントはアカウント認証が無効です。先に `enderpin permissions` の「Game permissions」でアカウント認証を有効にして保存します。`init` で作った環境は既定で有効です。

クライアントはMicrosoftの公開クライアントアプリケーションIDを使ってデバイスログインします。既定のIDは `f8d68570-e721-4aba-9c3e-1052d41e431a` です。別の登録済みIDを使う場合は `enderpin login --client-id YOUR_REGISTERED_CLIENT_ID` で上書きできます。

```sh
enderpin permissions  # quickで作った環境はアカウント認証を有効にする
enderpin login
enderpin launch --target client --connect localhost:25565
```

表示されたURLとコードを使ってブラウザでログインします。`--connect` はQuick Play対応版で利用でき、省略すると通常起動します。Microsoftの更新用資格情報はEnderpin専用のOS資格情報ストアへ保存し、構成・ロックには保存しません。`enderpin logout` で削除し、起動中の認証仲介も失効させます。既に確立したサーバー接続を切断する操作ではありません。

Minecraftのアクセストークンとチャット署名用秘密鍵はホスト側に保持し、ゲームにはプレースホルダーと用途を限定した認証ブリッジを渡します。対応するMinecraft・Java・authlibの組み合わせを検査し、未対応版では停止します。[認証とナレーターの対応範囲](sandbox.md#認証とナレーター) を参照してください。

起動は常に固定済みの構成を検証します。実行環境を更新する場合は `prepare --update`、Gitで戻したロックを再現する場合は `prepare --locked` を使います。Fabricの版を構成で指定するには各対象に `loader_version = "0.19.5"` のように記入します。
