# サンドボックスの権限

client / serverごとに、ゲーム全体へ許可する範囲を選びます。
すべてのMODが同じ許可を共有します。MODごとの権限宣言や、用途名を介したフォルダ割り当ては不要です。

## 試す

セットアップ後、次のコマンドで編集します。

```sh
enderpin permissions
enderpin --target server permissions
```

「Game permissions」で通信・ゲームフォルダの書き込み・クライアントのアカウント認証などを選び、
「External folders」で外部フォルダを追加します。Spaceで選択を切り替え、Enterで決定します。
メイン画面のEsc / Cancelは未保存の変更を破棄します。サブ画面のEscはその選択を取り消して戻ります。
高度なデスクトップ設定には、現在のOSが対応する項目だけを表示します。
別OS向けの非対応設定が残っている場合は「Reset unsupported desktop settings to defaults」で修正できます。
その操作を選ぶまでは、保存済みの禁止設定を自動的に緩めません。

「Save and review」で設定を保存し、権限とパッケージの一覧を確認して承認します。
「Save without approving」では設定だけを保存します。保存後の確認を断った場合も設定は残り、承認は保留になります。

```sh
enderpin permissions show       # 読みやすい一覧と承認状態
enderpin permissions approve    # 一覧を見て確認。ハッシュの入力は不要
enderpin launch                # 未承認なら、起動前に同じ確認を表示
enderpin permissions revoke     # 承認を取り消す
```

初回・設定変更時・MODの追加や更新時に承認が必要です。変更がなければ繰り返し確認しません。
承認は対象、OS、ワークスペースの場所、Minecraft・ローダー・ランタイムの固定情報、
導入パッケージのファイルハッシュ、共有管理下のゲーム側ファイルのパス・内容、設定、外部フォルダに結び付いています。
表示後と起動直前に内容を再確認します。起動直前は対象の実行ロックも保持します。
実行中の承認・設定・外部フォルダ変更は、ゲームを停止してから行います。

`quick` はファイルを取得する初期セットアップです。起動も権限の自動承認も行いません。
バニラの `launch --no-sandbox` は明示的にOS制限を無効にします（設定には保存しません）。
`--no-network` との併用はできません。Windowsでは子プロセス回収用のJob Objectを維持します。

### 自動化する場合

非対話では、調べた計画をハッシュで指定して承認する方法を残しています。
`--yes` では権限を自動承認しません。

```sh
enderpin permissions show --json
enderpin permissions approve <fingerprint>
```

JSONには完全な実効設定、パッケージのSHA-512、承認状態とfingerprintを含みます。
GUIは `sandbox::permissions::plan` と `Local` を利用できます。

## 設定ファイル

ゲーム設定は共有する `enderpin.toml`、承認と外部フォルダはPCごとの
`.enderpin/<client|server>/permissions.toml` に保存します。ローカルの権限ファイルはGitで共有しません。
標準値は省略でき、Enderpinも変更された項目だけを書き出します。

```toml
[client.sandbox]
account_authentication = false
read_only = ["mods", "resourcepacks", "shaderpacks"]

[server.sandbox]
network = false
```

標準では通信・ゲームフォルダへの書き込み・音声・デスクトップ連携・スキンキャッシュ・描画キャッシュを許可し、
ナレーターを無効にします。`init` のアカウント認証は有効、`quick` は認証を無効にした設定を明示的に保存します。
`game_write = false` はゲーム全体を書き込み禁止にします。
`read_only` は `saves`, `screenshots`, `resourcepacks`, `shaderpacks`, `mods`, `config`, `logs`,
`plugins`, `world` から選びます。標準以外のサーバーワールド名は `world` の制限に含まれません。
読み取り専用化はファイル内容の秘密化ではありません。読み取った内容を他の許可先へコピーできます。

スキンは `.enderpin/client/cache/enderpin-assets/skins` に保存します。
検証済みassetsは読み取り専用です。ゲーム全体の書き込みを止めても、許可済みスキンキャッシュは使えます。
Linuxの描画キャッシュは `.enderpin/client/cache/graphics`、macOSはOSユーザーのJava Metalキャッシュです。
macOSの描画キャッシュは同じOSユーザーの他のJavaアプリと共有されます。

`launch --no-network` は、その起動だけゲームの通信許可を狭めます。
通信許可はIPの送受信・LAN・待ち受けをまとめて扱います。ポート・ドメイン単位では制御しません。

## 外部フォルダ

編集画面のほか、コマンドでも既存のフォルダを直接指定できます。追加や変更だけでは承認しません。

```sh
enderpin permissions allow-folder /absolute/path/to/schematics          # 読み取り専用
enderpin permissions allow-folder /absolute/path/to/schematics --write  # 読み書き
enderpin permissions approve
enderpin permissions remove-folder /absolute/path/to/schematics         # 許可を削除。実フォルダは残す
```

ワークスペース・ホスト認証情報と重なるパス、ファイルシステムのルート、外部フォルダ同士の重複は拒否します。
起動時にはランタイムとの重複も検査します。正規化した絶対パスを保存し、起動前に再検証します。
MODのパス設定は自動変更しません。必要に応じてMOD側にも同じパスを指定してください。

旧形式の `[client.permissions.<mod>]` / `[server.permissions.<mod>]`、ローカルの `[bindings]` は廃止しました。
旧設定を使っている場合は該当のMOD宣言を削除し、古い `.enderpin/<client|server>/permissions.toml` を削除してから、
編集画面で外部フォルダを指定し直して承認してください。JAR内の `enderpin.permissions.json` は読みません。

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

権限は同じMinecraftプロセス全体に適用されます。MODごとの強制隔離は行いません。
ファイル検査とハッシュによる承認の対象はロックで管理するパッケージです。手動配置した管理外MODはこの検査に含みません。
実行中の追加要求・承認、ネットワークの宛先別制御、任意コードを実行するプラグイン機構は含めません。
