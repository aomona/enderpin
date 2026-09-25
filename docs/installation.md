# インストール

npm版とソース版から選べます。**このサイトの `files/`・`run/` 構成はmainブランチの仕様です。公開済みv0.1.3にはまだ含まれません。** 新構成を使う場合は、下の「最新のソース版」を利用してください。

npm版（v0.1.2から）は、次のコマンドでインストールします。

```sh
bun install -g enderpin
# または
npm install -g enderpin
enderpin --version
```

v0.1.3からは全5環境のネイティブバイナリをnpmパッケージに同梱し、OS・CPUに合うものを起動します。外部URLへの依存やインストール用スクリプト、`bun pm trust` は不要です。pnpmの `blockExoticSubdeps` を有効にしたまま使えます。

```sh
pnpm dlx enderpin@latest quick
# 版の確認
pnpm dlx enderpin@latest --version
```

`pnpm dlx enderpin` 単体はCLIの使い方を表示します。初期セットアップは末尾に `quick` を付けてください。

入口の実行にはNode.js 18以上を使います。Node.jsを入れていないBun環境では `bunx --bun enderpin quick` のように起動できます。更新は `bun update -g enderpin` または `npm install -g enderpin@latest` です。

ネイティブ実行ファイルを直接使う場合は、OS・CPUに合った `.tgz` のURLをBunに指定します。

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

表のファイル名に置き換えてください。特定の版に固定する場合は、`releases/latest/download/` を `releases/download/v0.1.3/` のような公開済みタグのパスに置き換えます。URL版からnpm版への切り替えは `bun remove -g enderpin` の後に `bun install -g enderpin` を実行してください。URL版の更新時も希望する版のURLで `bun install -g` を実行します。Bun 1.4.2では旧バイナリが残る場合を確認しています。`enderpin --version` で確認し、更新されていなければ `bun remove -g enderpin` の後に同じインストールコマンドを再実行してください。ゲームの保存データは削除しません。

このOS別パッケージには実行ファイルとJava/JNIブリッジを同梱しています。インストールにRust・JDK・Node.jsや `bun pm trust` は不要です。`enderpin` が見つからない場合は `bun pm bin -g` が示すディレクトリをPATHへ追加してください。Linuxのbubblewrapなど、ゲーム起動に必要なOS側の条件は引き続き必要です。Linuxの配布版はglibc向けで、Alpine Linux（musl）用ではありません。

## 最新のソース版

ソースからビルドする場合はRust 1.89以上、JDK 17以上、Cコンパイラーが必要です。

```sh
git clone https://github.com/aomona/enderpin.git
cd enderpin
cargo install --path . --locked
```
