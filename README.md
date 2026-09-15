# Enderpin

Minecraftのクライアントとサーバーの構成を、同じGitリポジトリで固定・再現するRustライブラリとCLIです。

初版として、パッケージの固定・同期、JavaとFabricの自動準備、認証付きクライアントとサーバーのサンドボックス起動を実装しています。macOSで実際のワールド参加まで確認しました。OSごとの検証範囲は [検証記録](docs/verification.md)、後続の実装順序は [PLAN.md](PLAN.md) を参照してください。

## インストール

[GitHub Releases](https://github.com/aomona/enderpin/releases) のOS・CPUに合った `.tgz` のURLを指定して、Bunでインストールできます。初回リリース公開後に利用できます。

```sh
# macOS / Apple Siliconの例
bun install -g https://github.com/aomona/enderpin/releases/latest/download/enderpin-darwin-arm64.tgz
enderpin --version
```

| 環境 | ファイル名 |
| --- | --- |
| macOS / Apple Silicon | `enderpin-darwin-arm64.tgz` |
| macOS / Intel | `enderpin-darwin-x64.tgz` |
| Linux / x64（glibc 2.35以上） | `enderpin-linux-x64.tgz` |
| Linux / ARM64（glibc 2.35以上） | `enderpin-linux-arm64.tgz` |
| Windows / x64 | `enderpin-win32-x64.tgz` |

表のファイル名に置き換えてください。特定の版に固定する場合は、`releases/latest/download/` を `releases/download/v0.1.0/` のような公開済みタグのパスに置き換えます。更新時も希望する版のURLで `bun install -g` を実行します。

パッケージには実行ファイルとJava/JNIブリッジを同梱しています。インストールにRust・JDK・Node.jsや `bun pm trust` は不要です。`enderpin` が見つからない場合は `bun pm bin -g` が示すディレクトリをPATHへ追加してください。Linuxのbubblewrapなど、ゲーム起動に必要なOS側の条件は引き続き必要です。Linuxの配布版はglibc向けで、Alpine Linux（musl）用ではありません。

ソースからビルドする場合はRust 1.89以上、JDK 17以上、Cコンパイラーが必要です。

```sh
cargo install --path . --locked
```

## ワンコマンドで起動

```sh
enderpin quick
# この起動だけサンドボックスを無効にする場合
enderpin quick --no-sandbox
```

実行時に公式メタデータの最新安定版を確認し、MinecraftとJavaを自動取得して、MOD・Fabricなしのクライアントを起動します。プレイヤー名は `Player`、ログイン不要のオフラインモードです。初回の準備と最新版の確認にはインターネット接続が必要です。認証が必要なマルチプレイサーバーやRealmsは利用できません。

サンドボックスは既定で有効です。`quick` の固定されたバニラ用設定（アカウント認証なし・外部フォルダ権限なし）には追加の承認操作は不要です。設定を手編集した場合は自動承認せず停止します。`--no-sandbox` はこの実行だけOSのファイル・通信制限を外します。

保存先はOSのEnderpinデータディレクトリ内の `quick/<Minecraft版>/` です。同じ版では設定とワールドを再利用し、新しい版は別フォルダに作成します。`-C DIRECTORY` を付けると `DIRECTORY/<Minecraft版>/` に変更できます。既存のMOD用ワークスペースは読み込みません。サーバーの準備・起動は行わず、`--target server` / `all` は拒否します。メモリ量は `--memory 4096` のようにMiBで指定できます。

## MOD用ワークスペースを試す

```sh
mkdir my-world
cd my-world
enderpin init --minecraft 1.21.1
enderpin add lithium --target all
enderpin add sodium --target client
enderpin search iris
enderpin list --target all
enderpin prepare --target all
```

`--target client` が標準です。`--target server` はサーバーだけ、`--target all` の追加は共通指定を編集して両側を同期します。検索は1つの対象を指定します。

別のPCではGitから構成・ロックを取得し、必要な対象だけを同期します。

```sh
enderpin sync --target server --locked
enderpin prepare --target server --locked
```

`--locked` は構成とロックの不一致を拒否します。通常の `sync` は手編集された構成を解決し、既存の固定版を維持します。最新版へ進める場合は `update` を使います。

## 起動する

`prepare` はMinecraft本体・Fabric・Temurin JDKの配布版とハッシュをロックし、現在のOS・CPU用のファイルを取得します。初回の両側準備後に、`enderpin.toml` と `enderpin.lock` をGitへ追加してください。別のPCでは必要な側だけを準備できます。

サーバーは [Minecraft EULA](https://aka.ms/MinecraftEULA) を確認し、同意する場合に明示して起動します。

```sh
enderpin launch --target server --accept-eula
```

同意はこのサーバーの `eula.txt` に保存します。ログを表示しながら `list`、`stop` 等のコンソール入力を使えます。Ctrl-Cは終了を要求し、2回目で強制停止します。保存完了まで自動的に待ち、時間だけを理由に強制終了しません。

クライアントはMicrosoftの公開クライアントアプリケーションIDを指定してデバイスログインします。現在、Enderpinには配布用の既定IDを同梱していません。下記の値には、Minecraft認証で利用できる登録済みのIDが必要です。

```sh
enderpin login --client-id YOUR_REGISTERED_CLIENT_ID
enderpin launch --target client --connect localhost:25565
```

表示されたURLとコードを使ってブラウザでログインします。`--connect` はQuick Play対応版で利用でき、省略すると通常起動します。Microsoftの更新用資格情報はEnderpin専用のOS資格情報ストアへ保存し、構成・ロックには保存しません。`enderpin logout` で削除し、起動中の認証仲介も失効させます。既に確立したサーバー接続を切断する操作ではありません。

Minecraftのアクセストークンとチャット署名用秘密鍵はホスト側に保持し、ゲームにはプレースホルダーと用途を限定した認証ブリッジを渡します。対応するMinecraft・Java・authlibの組み合わせを検査し、未対応版では停止します。[認証とナレーターの対応範囲](docs/sandbox.md#認証とナレーター) を参照してください。

起動は常に固定済みの構成を検証します。実行環境を更新する場合は `prepare --update`、Gitで戻したロックを再現する場合は `prepare --locked` を使います。Fabricの版を構成で指定するには各対象に `loader_version = "0.19.5"` のように記入します。

## サンドボックスと対応範囲

フォルダ別の書き込み禁止、MODの権限宣言、PCごとの承認と外部フォルダの割り当てに対応しています。[設定例とOSごとの制約](docs/sandbox.md) を参照してください。初回起動や権限・MODの変更時は起動前に確認し、非対話実行では `permissions show` → `permissions approve <fingerprint>` で事前承認します。

- macOSはSeatbelt、Linuxはbubblewrapとseccomp、WindowsはAppContainerとJob Objectを使います。通常起動への自動切り替えはありません。
- 対象のゲームディレクトリと専用一時ディレクトリを書込可能にし、共有Java・ライブラリ・キャッシュを読取専用にします。クライアントのassetsは検証して対象別キャッシュへ複製し、スキンの書き込み許可も分離します。
- 通信は既定で許可します。`launch --no-network` はゲームのIP通信を拒否します。ポートごとの制御やOSファイアウォールの変更は行いません。
- `--offline` は固定済みファイルとキャッシュだけで準備します。認証付きクライアントのセッション更新と承認済みの認証仲介には別途ホスト側のネットワークを使います。アカウントなしの公式デモは `launch --demo --offline --no-network` で試せます（事前に `prepare` が必要）。
- Linuxにはbubblewrapと利用可能なユーザー名前空間が必要です。デスクトップ起動はローカルX11またはWayland、必要に応じてPulseAudioとGPUを使います。
- MOD用の実行環境は現代のFabricプロファイルを対象とし、Minecraft 1.21.1で実測しています。`quick` はバニラクライアントを使います。未対応の旧式native形式やOSバージョン条件は理由を示して停止します。1.21.1のLinux ARM64クライアントは公式LWJGL nativeが適合しないため停止します（サーバーは対応）。
- Windows・Linuxの実JVMによる制限と認証ブリッジは検証済みですが、両OSのMinecraft画面・認証付きゲーム参加は未検証です。Windowsのloopback例外は自動追加しません。AppContainerの対象別プロファイルとファイル権限設定は起動後も残ります。

## 構成

```toml
format = 1
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

## 公開HTTPSファイル

```sh
enderpin add https://example.org/releases/example.jar --kind mod --name example
```

URLは実在する公開ファイルに置き換えてください。種類は `mod`、`resourcepack`、`shader`、`plugin` から明示します。必要な依存は別途追加します。取得したファイルのSHA-512とサイズを固定し、次回以降の不一致を拒否します。

更新する場合は、同じ名前で新しいURLを指定します。`update` はURL由来のファイルの内容を取り直しません。認証、非公開アドレス、HTTPへのリダイレクトには対応していません。1ファイルの上限は1 GiBです。

## 任意依存と個人用ignore

対話的な `add` では任意依存をSpaceキーで選べます。選択は構成に保存します。スクリプトでは `--optional project-a,project-b` で明示できます。必須依存を自動追加し、衝突時には停止します。過去版の組み合わせを総当たりする機能はありません。

`.enderpinignore` はGit管理外です。1行に1つの名前・IDを指定します。

```text
# このPCのクライアントだけ除外
client:iris
# 両側で除外
example-mod
```

他の有効なパッケージの必須依存を除外すると停止します。`sync --no-ignore` は個人設定を適用しません。ロックは完全な共有構成を保持します。

## コマンド

| コマンド | 動作 |
| --- | --- |
| `quick [--no-sandbox]` | 最新安定版のバニラクライアントをオフラインプレイヤーで準備・起動 |
| `init --minecraft VERSION` | 共有構成・ロック・Git除外設定を作成 |
| `add ID_OR_URL` | 追加して同期 |
| `remove NAME` | 指定スコープから削除して同期 |
| `search QUERY` | 対話検索・選択・追加 |
| `sync [--locked]` | 選んだ対象のファイルを同期 |
| `lock` | 解決結果を保存し、ゲームディレクトリは変更しない |
| `update` | 選んだ対象のModrinthパッケージを更新 |
| `restore` | 変更された管理対象ファイルをロックの内容へ明示的に復元 |
| `list` | ロックされたパッケージとロックの更新要否を表示 |
| `prepare [--locked] [--update]` | Minecraft・Fabric・Javaを固定して準備 |
| `login --client-id ID` / `logout` | OS資格情報ストアを使ったログイン・削除 |
| `permissions show / approve / revoke / bind / unbind` | 権限の確認・ローカル承認・外部フォルダの割り当て |
| `launch` | 選んだ片側をサンドボックス起動（`--memory` はMiB、既定2048） |

共通オプションは `-C DIRECTORY`、`--target client|server|all`、`--cache-dir DIRECTORY`、`--json`、`--no-interactive` です。`quick`・`login`・`launch` は `--json` 非対応、`launch` と `search` の対象は片側です。非対話環境の検索は一覧出力です。同期・復元の `--offline` は既存の固定内容とローカルキャッシュだけを使います。

## ファイルと保護

- `enderpin.toml` と `enderpin.lock` はGitへ追加します。
- `.enderpin/client/game/` と `.enderpin/server/game/` に対象ごとに配置します。
- `.enderpin/` と `.enderpinignore` はGit管理外です。
- ダウンロード済みファイルはOS標準のキャッシュディレクトリにハッシュで保存します。
- 管理外ファイルとの衝突、管理対象の手動変更、シンボリックリンク・Windows reparse pointによるパス逸脱を拒否します。
- 構成・ロック・ファイル変更をステージングし、復旧用ジャーナルを保存してから反映します。中断時は次の操作で元の状態へ戻します。復旧中に別の変更を見つけた場合はバックアップを保持して停止します。
- 以前の構成とロックをGitで戻して `sync --locked` すれば、そのファイルへ戻せます。配布元またはキャッシュから取得できることが前提です。

Enderpinで起動している対象への同期は拒否します。反対側は独立して操作できます。外部ランチャーで起動したゲームは検出しないため、終了してから同期してください。

## 検証

開発ビルドにはRustに加えてJDK 17以上（`java` / `javac` / `jar`）とCコンパイラーが必要です。Java/JNIブリッジをビルドして同梱します。クロスコンパイル時は対象OSのJDKヘッダーを `ENDERPIN_TARGET_JDK` で指定します。ゲーム用のJavaは引き続き別途固定・自動取得します。

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked

# 実ネットワークを使う追加検証。Python 3.11以上。
cargo build --locked
python3 tests/live_packages.py

# 実JVMでファイルと通信の許可・拒否を確認します。
ENDERPIN_TEST_JAVA_HOME=/absolute/path/to/jdk cargo test --locked -- --ignored --nocapture

# 固定済みワークスペースを複製してサーバーを起動・保存・終了します。
# EULAに同意する場合のみ実行してください。
python3 tests/live_server.py /path/to/workspace --accept-eula
```

通常テストは固定データと一時ディレクトリで実行します。実ネットワーク検証はModrinthの検索・導入、両側の配置、ハッシュ、キャッシュからの別環境再現、ignore、変更検出、URL導入を検証します。ゲームの起動やサンドボックスの強制を証明するテストではありません。

GitHub ActionsにはWindows・macOS・LinuxのRustチェックと実JVMのサンドボックス検証を定義しています。ワークフロー定義だけでは、各OSで実際に検証に成功したことを意味しません。[検証記録](docs/verification.md) に今回実行した範囲を記載しています。

### Bun配布パッケージ

Python 3.11以上とBunを使って、現在のOS・CPU向けのパッケージを作成・検証できます。

```sh
cargo build --release --locked
python3 scripts/package_bun.py target/release/enderpin
python3 tests/bun_install.py target/bun/enderpin-darwin-arm64.tgz
# 手元へインストールする場合
bun install -g ./target/bun/enderpin-darwin-arm64.tgz
```

Windowsでは実行ファイルを `target/release/enderpin.exe`、パッケージ名は上の表の環境に合わせて指定します。検証スクリプトは一時HTTPサーバーからBunで取得し、隔離したグローバル領域へインストールして、バージョン表示・ヘルプ・エラー終了・空白を含むパスでのワークスペース作成を確認します。普段のBunグローバル領域は変更しません。ゲーム起動の検証は含みません。

`.github/workflows/release.yml` は5環境でビルドとBunインストール検証を行います。手動実行ではActionsの成果物だけを作成します。`Cargo.toml` と一致する `vVERSION` タグをpushすると、全環境の成功後に `.tgz` を添付したReleaseの下書きを作成します。内容を確認して公開すると上のURLからインストールできます。対応するソースは同じタグのGitHubソースアーカイブから取得できます。

## ライセンス

[GPL-3.0-only](LICENSE)。[MonaLauncherからの再利用箇所](docs/third-party.md) を含みます。
