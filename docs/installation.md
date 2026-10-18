# インストール

GitHub Releaseの配布パッケージとソース版から選べます。v0.1.4には、このサイトの新CLIと `files/`・`run/` 構成が含まれます。旧版からの更新前に[移行手順](migration.md)を確認してください。ワールドの自動移動や旧コマンドとの互換対応は行いません。

全5環境のバイナリを同梱したパッケージを、次のURLからインストールします。

```sh
bun install -g https://github.com/aomona/enderpin/releases/download/v0.1.4/enderpin-0.1.4.tgz
# または
npm install -g https://github.com/aomona/enderpin/releases/download/v0.1.4/enderpin-0.1.4.tgz
enderpin --version
```

v0.1.3からは全5環境のネイティブバイナリをnpmパッケージに同梱し、OS・CPUに合うものを起動します。外部URLへの依存やインストール用スクリプト、`bun pm trust` は不要です。pnpmの `blockExoticSubdeps` を有効にしたまま使えます。

```sh
pnpm dlx https://github.com/aomona/enderpin/releases/download/v0.1.4/enderpin-0.1.4.tgz --help
# 版の確認
pnpm dlx https://github.com/aomona/enderpin/releases/download/v0.1.4/enderpin-0.1.4.tgz --version
```

引数なしではCLIの使い方を表示します。初期セットアップは `init` です。コマンドはその版の `--help` を参照してください。

入口の実行にはNode.js 18以上を使います。Node.jsを入れていないBun環境では `bunx --bun https://github.com/aomona/enderpin/releases/download/v0.1.4/enderpin-0.1.4.tgz --help` のように起動できます。更新時も希望する版のURLを指定してください。npmレジストリの公開版はv0.1.3のため、`enderpin@latest` ではv0.1.4を取得できません。

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

表のファイル名に置き換えてください。特定の版に固定する場合は、`releases/latest/download/` を `releases/download/v0.1.4/` のような公開済みタグのパスに置き換えます。URL版の更新時も希望する版のURLで `bun install -g` を実行します。Bun 1.4.2では旧バイナリが残る場合を確認しています。`enderpin --version` で確認し、更新されていなければ `bun remove -g enderpin` の後に同じインストールコマンドを再実行してください。ゲームの保存データは削除しません。

このOS別パッケージには実行ファイルとJava/JNIブリッジを同梱しています。インストールにRust・JDK・Node.jsや `bun pm trust` は不要です。`enderpin` が見つからない場合は `bun pm bin -g` が示すディレクトリをPATHへ追加してください。Linuxのbubblewrapなど、ゲーム起動に必要なOS側の条件は引き続き必要です。Linuxの配布版はglibc向けで、Alpine Linux（musl）用ではありません。

## 最新のソース版

ソースからビルドする場合はRust 1.89以上、JDK 17以上、Cコンパイラーが必要です。

```sh
git clone https://github.com/aomona/enderpin.git
cd enderpin
cargo install --path . --locked
```
