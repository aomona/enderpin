# 初版の検証記録

2026-09-14。ワークフローの定義と、実際に実行した検証を分けて記録する。

## 実行結果

| 環境 | 確認した範囲 | 結果 |
| --- | --- | --- |
| macOS / Apple Silicon / Rust 1.94 | fmt、全ターゲットClippy、通常テスト15件 | 成功 |
| Linux ARM64 / Docker / Rust 1.94 | 通常テスト15件 | 成功 |
| Windows x86_64 GNU向け / Linux上 | 全ターゲットのClippyとクロスコンパイル検査 | 成功。Windows実行ではない |
| macOS / 管理下Temurin JDK | 実JVMのファイル・通信制限、JVMによる実行中ロック保持 | 2件成功 |
| Linux ARM64 / OrbStack Ubuntu VM / 管理下Temurin JDK | bubblewrapとseccompによる実JVMのファイル・通信制限 | 成功 |
| macOS | 実Modrinth検索・導入・両側同期・ハッシュ・別環境オフライン再現・ignore依存・手動変更復元・HTTPS URL導入と削除 | 成功 |
| macOS、Linux ARM64 VM | Fabricサーバーの準備・起動・状態取得・コンソール・保存・正常終了・実行中同期拒否と終了後同期 | 両方成功 |
| macOS | Microsoftログイン、OS資格情報ストア保存、Fabricクライアントの描画・音声・認証付きサーバー参加 | 成功。利用者のワールド参加報告とサーバー側の参加ログで確認 |

ゲームで確認した固定構成はMinecraft 1.21.1、Fabric 0.19.5、Temurin `jdk-21.0.12.1+1`。共通のLithiumとクライアントのSodiumを同じ構成・ロックから配置した。サーバーは `online-mode=true`、`enforce-secure-profile=true` でlocalhostに待ち受けた。クライアントのスキンはゲーム内assetsへ保存され、共有キャッシュへの書込エラーがなくなったことも確認した。

クライアント認証の実測には、利用者の承認を得て既存MonaLauncherの公開クライアントIDを使用した。製品の既定IDとして同梱していない。資格情報はOSストアに保存し、この記録には含めない。接続テスト後はクライアントが終了し、サーバーも全ディメンションの保存と終了コード0を確認した。

## 制限プローブ

`tests/sandbox.rs` はゲーム用と一時用ディレクトリへの書込、固定ファイルの読取を許可し、それ以外のテストファイルの読取・書込、固定ファイルの変更を拒否することを実JVMで確認する。ネットワーク有効時のloopback通信と、無効時の接続拒否も検証する。macOSでは追加の子プロセス起動拒否も確認する。

Linuxでは合成ルートと `/tmp` 等の書込を拒否するよう明示的に読取専用へ再マウントし、許可した個別のbind mountだけを書込可能にした。ゲームを起動したスレッドは終了まで保持する。Docker内の入れ子の名前空間では実行条件を満たせなかったため、制限の実測にはUbuntu VMを使用した。システムJDKの外部シンボリックリンクへ許可を広げず、Enderpinが取得・固定したJDKで検証した。

これらは指定したプローブの結果であり、OSの脆弱性やあらゆるデスクトップサービス経由の操作に対する完全な隔離証明ではない。MODごとの権限分離や、ゲームのアクセストークンをMODから隠すbrokerは実装していない。

## 再実行

通常のRustチェック、公開配布元を使う `tests/live_packages.py`、実JVMのignoredテスト、実サーバーの `tests/live_server.py` の手順は [README](../README.md#検証) に記載している。

`live_server.py` は固定済みワークスペースを一時ディレクトリへ複製し、空きlocalhostポートで起動する。明示的な `--accept-eula` が必要。状態取得、実行中の同期拒否、`list`、`stop`、`world/level.dat` の保存、終了後の同期を検証する。このスクリプトだけではプレイヤー参加を検証しない。

CodeRabbitで未コミット変更をレビューし、指摘されたassets複製の途中ファイル対策と、保存中の30秒自動強制終了を修正した。最終コードで通常テストとClippyを再確認した。

## 未検証・後続

- Windowsのネイティブ実行・AppContainer強制・ゲーム起動。今回使えるWindows実機やVMはなかった。
- Linuxの実クライアント画面と認証付きゲーム接続。ARM64 VMではサーバーと制限プローブを確認した。
- GitHub Actionsの3 OSジョブの実行結果。定義済みだが、リモートへの公開・pushは行っていない。
- 上記以外のMinecraft・Fabric・Javaの組み合わせ、旧式native形式。
- Paper、NeoForge/Sinytra Connector、MonaLauncher組込、権限メタデータ。合意した後続段階で扱う。
