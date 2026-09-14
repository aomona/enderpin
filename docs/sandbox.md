# サンドボックスの権限

ゲーム全体の設定に、MODが必要とする権限を足して、起動前に利用者が承認します。
共有リポジトリやMODの宣言は要求であり、承認ではありません。

## 試す

```sh
enderpin --target client sync --locked
enderpin --target client prepare --locked
enderpin --target client permissions show
# 表示された fingerprint を指定する
enderpin --target client permissions approve <fingerprint>
enderpin --target client launch
```

対話的な `launch` は未承認の計画を表示して確認します。非対話実行では事前の承認が必要です。
`--yes` は権限の確認を省略しません。`permissions show --json` はCLIと同じ計画を構造化して返します。
GUIは `sandbox::permissions::plan` と `Local` を利用できます。

承認は `.enderpin/<client|server>/permissions.toml` に保存します。Gitで共有しません。
対象、OS、ワークスペースの場所、ゲーム・ローダー・ランタイム固定情報、導入対象のパッケージハッシュ、
要求の用途・内容、実効設定、外部フォルダの割り当てを照合します。変更時は再承認が必要です。
起動直前にも、対象の実行ロックを保持してファイルと承認を再確認します。

```sh
enderpin --target client permissions revoke
```

実行中の承認変更・取り消し・フォルダ再割り当ては拒否します。ゲームを停止してから変更します。

## ゲーム全体の設定

`enderpin.toml` のclient/serverそれぞれに指定できます。

```toml
[client.sandbox]
game_write = true
read_only = ["mods", "resourcepacks", "shaderpacks"]
network = false
audio = true
desktop_integration = true
skin_cache = true
graphics_cache = true
narrator = false
account_authentication = true
```

省略時は従来のゲーム書き込み・通信許可を維持します。`game_write = false` はゲーム全体を書き込み禁止にします。
`read_only` は `saves`, `screenshots`, `resourcepacks`, `shaderpacks`, `mods`, `config`, `logs`,
`plugins`, `world` から選びます。標準以外のサーバーワールド名は `world` の制限に含まれません。
読み取り専用化はファイル内容の秘密化ではありません。読み取った内容を他の許可先へコピーできます。

スキンはゲーム本体と別の `.enderpin/client/cache/enderpin-assets/skins` に保存します。
検証済みassetsは読み取り専用です。ゲーム全体の書き込みを止めても、許可済みスキンキャッシュは使えます。
Linuxの描画キャッシュは `.enderpin/client/cache/graphics`、macOSはOSユーザーのJava Metalキャッシュです。
macOSの描画キャッシュは同じOSユーザーの他のJavaアプリと共有されます。

`launch --no-network` は承認内容より通信を狭める起動指定です。
MODが必須の `network` を宣言している場合は、矛盾を示して停止します。
通信許可はIPの送受信・LAN・待ち受けをまとめて扱います。ポート・ドメイン単位では制御しません。

## MODの要求

対応MODは配布JAR/ZIPのルートに `enderpin.permissions.json` を置けます。
64 KiB以内、形式1、パッケージごとの要求はリポジトリ側と合わせて128件以内です。
ロックのハッシュを検証したファイルから読み、実行して問い合わせることはありません。
内包JARは個別に探索しません。外側の配布パッケージが必要な権限をまとめて宣言します。

```json
{
  "format": 1,
  "requests": [
    { "permission": "network", "reason": "共有サーバーへ接続するため" },
    { "permission": "folder-read", "folder": "schematics", "reason": "利用者が選んだ設計図を読み込むため" }
  ]
}
```

既存MODには共有リポジトリ側から要求を追記できます。キーはロック内のパッケージキー、ID、直接指定名です。
埋め込み宣言を削除・上書きせず、両方の要求を表示します。理由の文章はMOD作者・リポジトリ作者の説明であり、検証済みの事実ではありません。

```toml
[[client.permissions.litematica]]
permission = "folder-write"
folder = "schematics"
reason = "編集した設計図を保存するため"

[[client.permissions.litematica]]
permission = "directory-write"
directory = "config"
reason = "MOD設定を保存するため"
```

パッケージ名は実際の構成に合わせます。存在しないパッケージの要求は拒否します。
個人用ignoreで除外したパッケージの要求は適用しません。

要求は `network`, `audio`, `microphone`, `clipboard`, `desktop-integration`, `skin-cache`,
`graphics-cache`, `narrator`, `account-authentication`, `game-write`, `directory-write`, `folder-read`, `folder-write` です。
`directory-write` は指定ディレクトリを `read_only` から外します。ゲーム全体が書き込み禁止なら、
別途 `game-write` を要求しなければ停止します。宣言した権限はこの段階ではすべて必須です。
拒否する場合は起動せず、そのMODを除外するか構成を見直します。

## 外部フォルダ

MODは用途を表す名前だけを指定します。実際のホストパスは、そのPCの利用者が割り当てます。

```sh
enderpin --target client permissions bind schematics /absolute/path/to/schematics
enderpin --target client permissions show
enderpin --target client permissions approve <fingerprint>
# 不要になった割り当てを外す
enderpin --target client permissions unbind schematics
```

既存のディレクトリだけを指定できます。ワークスペースやランタイムと重なる許可は拒否します。
同じ名前への読み取り・書き込み要求は書き込み許可へ統合し、要求元を両方表示します。
割り当ては環境変数でMODに渡されません。MOD自身の設定にも必要に応じて同じパスを指定してください。
権限付与はOSによるアクセス許可であり、MODのパス設定変更とは別です。

## OSで強制できる範囲

| 項目 | macOS | Linux | Windows |
| --- | --- | --- | --- |
| ゲーム・外部フォルダの読み書き | Seatbelt | bind mount | AppContainer ACL |
| IP通信 | Seatbelt | network namespace + seccomp | AppContainer capabilities |
| 音声出力の禁止 | CoreAudio許可を外す | PulseAudio socketを渡さない | 個別指定不可 |
| マイクの個別指定 | 対応。別途OSの同意が必要 | 音声許可時は録音も含まれる | 個別指定不可 |
| クリップボードの個別指定 | 対応 | display protocolから独立した禁止は不可 | 個別指定不可 |
| IME・全画面の追加連携 | 個別指定可能 | 個別の禁止は不可 | 個別の禁止は不可 |
| 描画キャッシュの禁止 | Java Metal cacheの許可を外す | 専用cacheを読み取り専用化 | 個別の禁止は不可 |

`microphone` と `clipboard` を省略すると、macOSでは個別許可なし、Linux/WindowsではOS・デスクトップ接続の制約に従います。
Linux/Windowsで省略している項目を「拒否できている」とは表示しません。
例えばLinuxで `audio = true` と `microphone = false` を組み合わせると停止します。
Windowsでは `microphone` / `clipboard` の明示指定、`audio = false` / `desktop_integration = false` /
`graphics_cache = false` を拒否します。ヘッドレスサーバーにはデスクトップ権限を付けません。

LinuxはWaylandが指定された環境ではWaylandを優先し、失敗時にX11へ自動切り替えしません。
X11は他クライアントへのアクセス、Waylandはcompositor protocolへのアクセスを伴います。
PulseAudioも出力専用の接続ではありません。これらの制約を承認画面に表示します。
WindowsにはAppContainer固有のプロファイル保存領域もあります。

読み取り専用のMODディレクトリは書き込み・削除・名前変更を禁止します。
事前に存在するシンボリックリンク、Windows reparse point、ハードリンク、特殊ファイルをデータ領域で拒否します。
Windowsは権限の組み合わせを変えたときに別のAppContainer SIDを使い、以前のSIDのACLを新しいプロセスに引き継ぎません。
古いプロファイルとACLエントリの自動掃除はまだ行いません。

## 認証とナレーター

MonaLauncherの認証brokerとナレーターbridgeを [GPLv3で再利用](third-party.md) しています。
`account_authentication = true` のクライアントはゲームへプレースホルダーだけを渡し、Minecraftのアクセストークンと
チャット署名用の秘密鍵をホスト側に保持します。Microsoftの更新資格情報はOSストアだけに保持します。
Unixは継承したsocket pair、Windowsは接続済みのpipe handleを使います。汎用HTTPプロキシや任意データの署名APIではありません。
ゲームの `network` が無効でも、承認済みのアカウント認証はホスト側で実行できます。
`logout` やアカウント変更で仲介を失効させます。既に確立したサーバー接続を切断する操作ではありません。

アダプターはMinecraft 1.21.1 / 1.21.8（Java 21）、26.2（Java 25）、Fabric 0.19.5に限定し、
公式authlibのハッシュを検査します。未対応版やブリッジ失敗では起動を停止し、生トークンへ戻しません。
バージョン別・OS別の実測範囲は [検証記録](verification.md) を参照してください。
旧方式の `profilekeys` キャッシュは仲介へ切り替える起動で削除し、バックアップしません。
`account_authentication = false` はアカウントを使わない起動で、認証必須サーバーへの自動参加はできません。

`narrator = true` はゲームのナレーター要求だけをホストの音声エンジンへ渡します。
WindowsはSAPI、macOSはAVSpeechSynthesizer、Linuxは `/usr/bin/espeak-ng` を使います。
テキスト長・要求頻度・キューを制限し、ゲーム終了時に停止します。Linuxでは事前にespeak-ngの導入が必要です。
この設定は他MOD独自の音声合成を識別して禁止する機能ではありません。

## 境界と次の段階

MODごとの要求は、同じMinecraftプロセス全体に対する許可へ統合されます。
同じJVM内の別MODもその権限を利用できます。要求元の表示はMOD単位の強制隔離を意味しません。
宣言の検査とハッシュによる承認の対象はロックで管理するパッケージです。手動配置した管理外MODはこの検査に含みません。
実行中の追加要求・承認、ネットワークの宛先別制御、任意コードを実行するプラグイン機構は含めません。
