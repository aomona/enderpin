# コマンド

| コマンド | 動作 |
| --- | --- |
| `quick [--server] [--version VERSION] [--loader vanilla\|fabric] [--mods IDS]` | 検索付き初期セットアップ。`--server` は両側を準備。ゲームは起動しない |
| `init --minecraft VERSION` | 共有構成・ロック・Git除外設定を作成 |
| `add ID_OR_URL` | 追加して同期 |
| `remove NAME` | 指定スコープから削除して同期 |
| `search QUERY` | 対話検索・選択・追加 |
| `sync [--locked]` | 選んだ対象のファイルを同期 |
| `lock` | 解決結果を保存し、ゲームディレクトリは変更しない |
| `update` | 選んだ対象のModrinthパッケージを更新 |
| `restore` | 管理パッケージをロックへ、共有ファイルを現在の `files/` へ明示的に復元 |
| `list` | ロックされたパッケージとロックの更新要否を表示 |
| `prepare [--locked] [--update]` | Minecraft・Fabric・Javaを固定して準備 |
| `login --client-id ID` / `logout` | OS資格情報ストアを使ったログイン・削除 |
| `permissions [show / approve / revoke / allow-folder / remove-folder]` | 対話編集・権限の確認と承認・外部フォルダの許可 |
| `launch` | 選んだ片側をサンドボックス起動（`--memory` はMiB、既定2048） |

共通オプションは `-C DIRECTORY`、`--target client|server|all`、`--cache-dir DIRECTORY`、`--json`、`--no-interactive` です。`quick`・`login`・`launch` は `--json` 非対応、`launch` と `search` の対象は片側です。非対話環境の検索は一覧出力です。同期・復元の `--offline` は既存の固定内容とローカルキャッシュだけを使います。
