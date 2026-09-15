# 検証記録

2026-09-15更新。quick起動、サンドボックス拡張、初版の実測を区別して記録する。

## pnpm対応のバイナリ同梱パッケージ（v0.1.3、2026-09-15）

- pnpm 12.4.2で `blockExoticSubdeps: true` を指定し、公開済み0.1.2の `ERR_PNPM_EXOTIC_SUBDEP` を再現。GitHub URLをoptional dependencyにしていたことが原因。
- 0.1.3ではOS別バイナリ5種類を1つのnpmアーカイブへ同梱。依存・インストール用スクリプトは0件。公開物は9ファイル、圧縮後20,598,419バイト。
- [Release Actions](https://github.com/aomona/enderpin/actions/runs/34969035608)：Linux x64/ARM64、macOS Intel/ARM64、Windows x64の全5環境で、ネイティブビルド後に同梱版のnpm/Bun導入とpnpm dlxを検証。すべて成功。
- [Check Actions](https://github.com/aomona/enderpin/actions/runs/34969035671)：3 OSのRustテスト・fmt・Clippy・実JVMサンドボックス確認が成功。ローカルの通常テストも46件成功。
- 全5環境のアーカイブがないと公開用パッケージの作成を拒否することを確認。`--allow-partial` のローカル検証用は `private: true` で生成。
- CodeRabbitの変更8ファイルのレビューは指摘0件。
- npm公開処理の完了後、`latest = 0.1.3` と配布アーカイブのintegrityを確認。`tests/npm_install.py target/npm-release-v0.1.3/enderpin-0.1.3.tgz --registry` で公開版を名前から取得し、npm/Bun導入とpnpm dlxが成功した。pnpmの `blockExoticSubdeps` は有効のまま検証した。

## npm配布パッケージ（v0.1.2、2026-09-15）

- macOS ARM64でnpm 11.19.0・Bun 1.4.2の隔離グローバルインストールを実行。インストール用スクリプトを無効にして、OS別バイナリの起動、0.1.2の版表示、CLIのエラー終了コード、空白を含むパスでの初期化を確認。
- `tests/npm_install.py` はnpm/Bunのprefix・グローバル設定・キャッシュを一時領域に置く。ローカル検証用URLは一時コピーにだけ設定し、公開アーカイブは同じ版のGitHub HTTPS URLだけを含むことを検査する。
- Node.jsのディレクトリをPATHから外し、Bunの `bunx --bun enderpin --version` でも実行を確認。`bun --bun enderpin` はこのグローバル配置では解決できなかったため、案内を修正した。
- `npm publish --dry-run` で公開対象が起動用JavaScript・package.json・README・LICENSEの4ファイルであることを確認。実公開の完了を示すチェックではない。
- Rust通常テスト46件、Clippy、fmt、releaseビルド成功。CodeRabbitによる8ファイルの配布差分レビューは指摘0件。その後、Bunだけで実行する検証とドキュメントを上記の実測結果へ修正した。

- [v0.1.2 Release Actions](https://github.com/aomona/enderpin/actions/runs/34966610213)：Linux x64/ARM64、macOS ARM64/Intel、Windows x64の全5環境でビルドとnpm/Bun導入を確認。[Check Actions](https://github.com/aomona/enderpin/actions/runs/34966610639)も3 OSすべて成功。Windowsのテスト用cmd.exe引数の二重引用を修正した後の結果。
- [GitHub Release v0.1.2](https://github.com/aomona/enderpin/releases/tag/v0.1.2)を公開し、そのアーカイブからnpm/Bun導入を再確認。[npm enderpin](https://www.npmjs.com/package/enderpin)へ0.1.2を公開、`latest = 0.1.2` と公開物のintegrityを確認した。
- 公開後は `tests/npm_install.py target/npm-release/enderpin-0.1.2.tgz --registry` で、npm/Bunがパッケージ名から取得して起動することをmacOS ARM64で確認した。
- このMacのVite Plus製npmラッパーは、`--prefix` 指定時もユーザーのbinにリンクを作った。そのテスト由来のリンクだけを削除し、検証スクリプトは実際のNode.js/npm CLIを直接呼ぶ形へ修正。再検証で追加リンクができないことと、元のBun版0.1.1が引き続き選ばれることを確認した。
- npmの起動ラッパーを通した `tests/live_setup.py` も成功。実TUIの検索・MOD選択・ダウンロード・両側配置・中止・ゲーム未起動を確認した。今回npm版による実ゲームの起動は検証対象に含めていない。

## 検索付きquickセットアップ（開発版、2026-09-15）

`quick` をゲーム起動から初期セットアップに変更。単体はクライアント、`--server` は両側のMinecraft・Java・ローダー・MODを準備する。Minecraft版とローダーは文字入力で絞り込む選択画面、MODはModrinth検索と選択・取り消し・ページ移動を使う。候補ローダーは現在Vanilla/Fabric。既存の設定は上書きしない。

- macOS Apple Silicon：`cargo test --locked` 46件、Clippy（警告をエラー扱い）、fmt、releaseビルド成功。実JVM用4件はこのテストではignored。
- `python3 tests/live_setup.py --binary target/release/enderpin`：擬似端末から版を検索し、1.21.1・Fabricを選択。Lithiumの検索・追加・取り消し・再追加とダウンロードを確認。クライアントのロックとMODの実ファイルを確認。
- 同スクリプトで `--server --version 1.21.1 --loader fabric --mods lithium,sodium --no-interactive` を実行。両側のランタイムを固定し、Lithiumは両側、Sodiumはクライアントだけに配置。
- セットアップでMinecraftを起動しないこと、権限を自動承認しないこと、EULA同意ファイルを作らないことを確認。Esc中止では空ディレクトリにファイルが残らない。
- 更新した `tests/live_server.py --quick --version 26.2 --accept-eula --binary target/release/enderpin` では、セットアップ後に明示的に権限承認・`launch` を実行。サーバーの応答・コンソール・保存・正常終了を確認。
- CodeRabbitの追跡済み変更7ファイルのレビューは軽微な指摘1件。両側の初回ダウンロードを検証するスクリプトの上限時間を600秒から1800秒へ延長した。
- Windows/LinuxのTUI操作は未実測。公開済みv0.1.1にはこのセットアップ動作は含まれない。

## 直下のclient/server構成（開発版、2026-09-15）

通常ワークスペースとquickのゲーム保存先を `client/`・`server/` に変更。quickは実行ディレクトリ（または `-C` 指定先）に共通の設定・ロックを作り、両側のゲームディレクトリを使う。内部状態は `.enderpin/` に残す。新形式は `format = 2`、旧形式の自動移行は行わない。

- macOS Apple Siliconで `cargo test --locked` 46件成功、実JVM用4件はignored。Clippy（警告をエラー扱い）、fmt、releaseビルド成功。
- 新配置のMOD同期・復元、反対側のファイルを指定した状態の拒否、許可された同期先以外の拒否、設定だけ複製した空ワークスペースでのディレクトリ作成を検証。
- `tests/live_server.py --quick --version 26.2 --accept-eula --binary target/release/enderpin`：`-C` なしで一時ディレクトリをcwdにして起動。`client/`・`server/`・設定ファイルの直下配置、ポート応答、コンソール操作、実行中sync拒否、`server/world/level.dat` の保存と正常終了を確認。
- 一時ディレクトリで `quick --version 26.2 --no-interactive` を実行。Seatbelt内でクライアントを起動し、`client/logs/latest.log`、音声初期化・アトラス生成ログ、反対側のファイル保持、Ctrl-Cによる終了を確認。画面とプレイは確認していない。
- CodeRabbitの軽微な指摘1件（設定だけ複製したワークスペースのゲームディレクトリ不足）は、共通の同期処理で作成する変更と、その回帰テストで対応。
- この配置変更についてWindows/Linuxの実ゲームは未検証。公開済みv0.1.1には含まれない。

## quickサーバー・ポート・版指定と公開後インストール（2026-09-15）

v0.1.1に含むサーバー・版指定・ポート指定を、`codex/quick-server` でmacOS / Apple Siliconにて確認した。

- `cargo test --locked`: 44件成功、実JVM用4件は既定でignored。
- `cargo clippy --all-targets --locked -- -D warnings`、`cargo fmt --check` 成功。
- `tests/live_server.py --quick --accept-eula --binary target/release/enderpin`: 最新安定版26.2をSeatbelt内で実起動。任意のポートを `--port` で指定し、状態取得、コンソールlist、実行中のsync拒否、stop、ワールド保存、終了後のsyncに成功。
- 同スクリプトに `--version 1.21.1 --no-sandbox` を指定した経路も同じ検証に成功。
- 検証用サーバーは一時ワールドとループバック接続を使用。両経路とも `online-mode=true`、明示同意後の `eula=true` を確認。認証済みプレイヤーの参加は未検証。製品の接続設定はMinecraft標準を保持する。
- 26.2はDoneログ直後に状態取得接続を閉じる場合があったため、テストは最大10秒、状態情報の準備を待つ。
- CodeRabbit CLI 0.7.6による変更差分レビューは軽微な指摘1件。テスト側で取得した最新バージョンを起動引数にも固定し、取得間の最新版変更による不一致を防いだ。修正後の実サーバー検証も成功。
- Windows/Linuxでの新しいquickサーバーの実起動は未検証。

v0.1.0は5環境のRelease Actions成功後に公開した。公開URLからmacOS ARM64パッケージを取得し、通常のグローバル領域と隔離領域の両方でインストール・バージョン表示を確認した。

旧Bun検証は `--config` の `globalDir` だけではグローバル依存ファイルを隔離できず、このMacに一時HTTP URLを残していた。Enderpinの依存先のみ公開URLへ修復した。検証スクリプトは `BUN_INSTALL_GLOBAL_DIR` / `BUN_INSTALL_BIN` / `BUN_INSTALL_CACHE_DIR` を[Bun公式の環境変数](https://bun.sh/docs/runtime/bunfig)として明示し、一時領域のpackage.jsonにEnderpinだけが入ったことも確認する。下の旧記録にある「隔離」はこの修正前には不完全だった。

## quick起動の実測（2026-09-15）

macOS / Apple Silicon / Rust 1.94 / Bun 1.4.2で確認した。

| 確認 | 結果 |
| --- | --- |
| `cargo fmt --check`、全ターゲットClippy、通常テスト43件 | 成功 |
| 実JVMの既存の制限・認証ブリッジ・実行ロック保持と、新しい非サンドボックス起動の外部ファイル書込テスト | 4件成功 |
| `quick --no-interactive` | 安定版26.2を取得し、Seatbelt内で `Player` として起動。描画・音声初期化ログと利用者提供のタイトル画面を確認 |
| `quick --no-sandbox --no-interactive` | `Unconfined client started`、`Setting user: Player`、アトラス生成、音声初期化のログを確認。画面キャプチャはツール側で失敗したため、この経路の画面は直接確認していない |
| ロック改変への対応 | quick専用ロックのメタデータURLを `https://example.invalid/quick-lock-regression.json` に変更してから起動。公式Mojang URLへ再解決され、26.2の起動に成功 |
| Bun配布パッケージ | 更新したreleaseビルドを梱包し、一時HTTP URLから隔離したグローバル領域へインストール。CLI検証成功 |
| CodeRabbit CLI 0.7.6 | 公式配布アーカイブとのSHA-256一致を確認して未コミット差分をレビュー。ロック改変に関する1件を修正し、再レビューは指摘0件 |

quick用ロックはクライアントのみ、ローダーは `vanilla`、Fabricライブラリは0件だった。サーバーは起動していない。
オフラインモードなので、ゲームログにはアカウント属性・Realmsの認証エラーが出る。タイトル画面の起動失敗ではない。
検証用クライアントはCtrl-Cで停止した。ワールド生成・プレイとWindows/Linuxの今回の変更は未実測。

最新版は実行時に公式メタデータの `latest.release` を使う。固定データのテストではsnapshotを選ばないこと、
サーバー指定、改変したquick構成・クライアントロックを拒否することも確認した。

再実行は `enderpin quick` と `enderpin quick --no-sandbox`。実JVMの追加テストは下の「再実行」のコマンドを使う。

## サンドボックス拡張の実測

| 環境 | 確認した範囲 | 結果 |
| --- | --- | --- |
| macOS / Apple Silicon / Rust 1.94 | fmt、全ターゲットClippy、通常テスト41件 | 成功 |
| Linux ARM64 / Docker / Rust 1.94 | 全ターゲットClippy、通常テスト41件、Java/JNIビルド | 成功 |
| macOS / Temurin 21.0.7 | 実JVMのファイル・通信制限、認証JNI、ナレーターJavaブリッジ、JVMによる実行中ロック保持 | 成功 |
| Linux ARM64 / OrbStack Ubuntu VM / 管理下Temurin 21.0.12.1+1 | bubblewrap・seccompの実JVM制限、認証JNI、ナレーターJavaブリッジ | 成功 |
| Windows / GitHub Actions / Temurin 21 | fmt、全ターゲットClippy、通常テスト42件、認証JNI、ナレーターJavaブリッジ、AppContainerのファイル・通信制限と権限の遷移 | 成功 |
| macOS / Fabric 1.21.1 | ホスト認証を使った実クライアントの描画・音声初期化と認証付きサーバー参加 | 成功。サーバーの参加ログと秘密鍵キャッシュがないことを確認 |
| macOS / Fabric 1.21.1 | サーバーの状態取得・コンソール・保存・正常終了・実行中同期拒否と終了後同期 | 成功 |

[GitHub Actions run 34846902368](https://github.com/aomona/enderpin/actions/runs/34846902368) はコミット `1bbaa89` の3 OSすべてで成功した。Windowsでは、継承を無効にした既存ファイルにも直接の読取権限を適用する修正後、読取専用化・取消し・再許可の全シナリオが成功した。

通常テスト（macOS/Linux 41件、Windows 42件）には、権限宣言の読取とハッシュ照合、ローカル割り当て、変更時の承認失効、未対応の拒否設定、ハードリンク拒否、認証の失効・入力境界・署名の用途制限、ナレーターの入力・キュー・頻度制限を含む。

### 制限と認証のプローブ

`tests/sandbox.rs` は書込可能なゲーム・外部フォルダから、MODサブフォルダと外部フォルダの読取専用化、ゲーム全体の読取専用化と外部フォルダの取消し、元の権限への復帰を順番に検証する。読取専用MODの書込・削除・名前変更、許可していないファイルの読取・書込、Javaの変更が拒否されることを実JVMで確認する。ネットワーク有効時のloopback通信と無効時の接続拒否、macOSでは追加の子プロセス起動拒否も検証する。

`src/auth/broker/native_tests.rs` は実際のJavaエージェントとJNIをサンドボックス内で使う。テスト専用鍵で生成したチャット署名の検証、秘密鍵をエクスポートしないこと、別ハンドルの拒否、仲介の通信拒否を確認する。ナレーターJavaブリッジも許可時・無効時の出力を検査する。音声エンジンから実際に聞こえることの確認ではない。

LinuxではDockerをビルドに使い、OS制限はOrbStack VMで実測した。合成ルートと `/tmp` 等を読取専用にし、明示したbind mountだけを書込可能にする。seccompのファイル記述子と認証socketの継承スロットを分離し、bubblewrapが継承socketをJVMへ渡せることを確認した。

### 実ゲームでの認証

固定構成はMinecraft 1.21.1、Fabric 0.19.5、Temurin `jdk-21.0.12.1+1`。共有LithiumとクライアントSodiumを同じ構成・ロックから配置した。localhostのサーバーは `online-mode=true`、`enforce-secure-profile=true`。ゲームへアクセストークンを渡す方式への切り替えは行わず、ホスト認証ブリッジで参加した。

実測には利用者の承認を得てMonaLauncherの公開クライアントIDとOSストアのアカウントを使用した。製品の既定IDとしては同梱していない。資格情報はこの記録に含めない。検証用ワークスペースは一時ディレクトリへ複製し、終了時にクライアントとサーバーを停止した。プレイヤーのチャット送信は実行していないため、実ゲームでの署名付きチャットの相互運用まで成功したとは扱わない。

## 再実行

ビルドにはJDK 17以上とCコンパイラーが必要。Java/JNIブリッジはビルド時に同梱し、通常利用時にはコンパイラーを必要としない。

```sh
cargo fmt --check
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
ENDERPIN_TEST_JAVA_HOME=/absolute/path/to/jdk cargo test --locked -- --ignored --nocapture
cargo build --locked
python3 tests/live_server.py /path/to/locked/workspace --accept-eula
python3 tests/live_client.py /path/to/locked/workspace --accept-eula --use-account
```

実ゲームのスクリプトには固定済みファイル・実行環境キャッシュが必要。`live_client.py` はデスクトップとログイン済みアカウントも必要とする。両スクリプトとも権限要求や外部フォルダがない検証用構成だけを自動承認し、MODの要求は自動承認しない。`live_server.py` はプレイヤー参加を検証しない。

CodeRabbitのレビューを実施し、権限表示の制御文字除去、無音設定時のPulseAudio環境変数処理、認証状態の読取エラー処理、Windows検証用クライアントの終了処理を修正した。未対応の必須権限を黙って無視する提案は採用せず、理由を示して起動を停止する動作を維持した。

## 初版で確認済みの範囲

初版ではmacOS上で実Modrinth検索・導入・両側同期・ハッシュ・別環境でのオフライン再現・ignore依存・手動変更の復元・HTTPS URL導入と削除を確認した。Fabricサーバーの起動・コンソール・保存・終了はmacOSとLinux ARM64 VMで確認した。これらの配布元・全ゲーム操作をサンドボックス拡張の各修正ごとに繰り返したわけではない。

## 未検証・後続

- WindowsとLinuxの実Minecraftクライアント画面、認証付きサーバー参加。CIの実JVMプローブとゲーム画面の検証は別。
- 各OSのホスト音声エンジンによる実際の読み上げ、マイク・クリップボード・全デスクトップサービスの操作。対応と制約は [sandbox.md](sandbox.md) を参照。
- 実ゲームの署名付きチャット送信。Minecraft 1.21.8 / 26.2のアダプターは移植済みだが、Enderpinからの実ゲーム接続は未検証。
- MOD単位の強制隔離、実行中の権限追加。承認した権限はMinecraftプロセス全体で共有する。
- Paper、NeoForge/Sinytra Connector、MonaLauncherへの組込は後続段階。

この記録は指定した条件・プローブの結果であり、OSの脆弱性やあらゆるデスクトップサービスを含む完全な隔離証明ではない。
