# 検証記録

2026-09-15更新。quick起動、サンドボックス拡張、初版の実測を区別して記録する。

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
