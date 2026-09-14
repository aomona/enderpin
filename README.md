# Enderpin

Minecraftのクライアントとサーバーの構成を、同じGitリポジトリで固定・再現するRustライブラリとCLIです。

開発中です。現在はパッケージ管理部分を実装しています。初版の完成条件にはFabricの両側起動・Javaの準備・サンドボックスも含まれ、それらはまだ開発途中です。実装範囲と順序は [PLAN.md](PLAN.md) を参照してください。

## 試す

Rust 1.89以上が必要です。

```sh
cargo install --path . --locked

mkdir my-world
cd my-world
enderpin init --minecraft 1.21.1
enderpin add lithium --target all
enderpin add sodium --target client
enderpin search iris
enderpin list --target all
```

`--target client` が標準です。`--target server` はサーバーだけ、`--target all` の追加は共通指定を編集して両側を同期します。検索は1つの対象を指定します。

別のPCではGitから構成・ロックを取得し、必要な対象だけを同期します。

```sh
enderpin sync --target server --locked
```

`--locked` は構成とロックの不一致を拒否します。通常の `sync` は手編集された構成を解決し、既存の固定版を維持します。最新版へ進める場合は `update` を使います。

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
| `init --minecraft VERSION` | 共有構成・ロック・Git除外設定を作成 |
| `add ID_OR_URL` | 追加して同期 |
| `remove NAME` | 指定スコープから削除して同期 |
| `search QUERY` | 対話検索・選択・追加 |
| `sync [--locked]` | 選んだ対象のファイルを同期 |
| `lock` | 解決結果を保存し、ゲームディレクトリは変更しない |
| `update` | 選んだ対象のModrinthパッケージを更新 |
| `restore` | 変更された管理対象ファイルをロックの内容へ明示的に復元 |
| `list` | ロックされたパッケージとロックの更新要否を表示 |

全コマンドで `-C DIRECTORY`、`--target client|server|all`、`--cache-dir DIRECTORY`、`--json`、`--no-interactive` を利用できます。非対話環境の検索は一覧出力です。同期・復元の `--offline` は既存の固定内容とローカルキャッシュだけを使います。

## ファイルと保護

- `enderpin.toml` と `enderpin.lock` はGitへ追加します。
- `.enderpin/client/game/` と `.enderpin/server/game/` に対象ごとに配置します。
- `.enderpin/` と `.enderpinignore` はGit管理外です。
- ダウンロード済みファイルはOS標準のキャッシュディレクトリにハッシュで保存します。
- 管理外ファイルとの衝突、管理対象の手動変更、シンボリックリンク・Windows reparse pointによるパス逸脱を拒否します。
- 構成・ロック・ファイル変更をステージングし、復旧用ジャーナルを保存してから反映します。中断時は次の操作で元の状態へ戻します。復旧中に別の変更を見つけた場合はバックアップを保持して停止します。
- 以前の構成とロックをGitで戻して `sync --locked` すれば、そのファイルへ戻せます。配布元またはキャッシュから取得できることが前提です。

現在のパッケージ管理CLIにはゲームの実行中管理がまだないため、外部ランチャーで起動したゲームは終了してから同期してください。

## 検証

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked

# 実ネットワークを使う追加検証。Python 3.11以上。
cargo build --locked
python3 tests/live_packages.py
```

通常テストは固定データと一時ディレクトリで実行します。実ネットワーク検証はModrinthの検索・導入、両側の配置、ハッシュ、キャッシュからの別環境再現、ignore、変更検出、URL導入を検証します。ゲームの起動やサンドボックスの強制を証明するテストではありません。

GitHub ActionsにはWindows・macOS・LinuxのRustチェックを定義しています。ワークフロー定義だけでは、各OSで実際に検証に成功したことを意味しません。
