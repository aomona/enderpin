# MODを追加・更新する

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


## 更新・削除する

```sh
enderpin update --target all
enderpin remove sodium --target client
```

`update` は選んだ対象のModrinthパッケージを更新します。通常の `sync` は既存の固定版を維持します。依存されているパッケージは必要な限り残ります。更新後は差分を確認し、設定とロックを一緒にコミットしてください。
