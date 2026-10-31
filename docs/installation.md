# インストール

GitHub Releaseの配布パッケージとソース版から選べます。v0.2.0は、このサイトで説明する名前付きターゲットとformat 4に対応しています。旧版からの更新前に[移行手順](migration.md)を確認してください。ワールドの自動移動や旧コマンドとの互換対応は行いません。

全5環境のバイナリを同梱したパッケージを、次のURLからインストールします。

```sh
bun install -g https://github.com/aomona/enderpin/releases/download/v0.2.1/enderpin-0.2.1.tgz
# または
npm install -g https://github.com/aomona/enderpin/releases/download/v0.2.1/enderpin-0.2.1.tgz
enderpin --version
```

v0.1.3からは全5環境のネイティブバイナリをnpmパッケージに同梱し、OS・CPUに合うものを起動します。外部URLへの依存やインストール用スクリプト、`bun pm trust` は不要です。pnpmの `blockExoticSubdeps` を有効にしたまま使えます。

```sh
pnpm dlx https://github.com/aomona/enderpin/releases/download/v0.2.1/enderpin-0.2.1.tgz --help
# 版の確認
pnpm dlx https://github.com/aomona/enderpin/releases/download/v0.2.1/enderpin-0.2.1.tgz --version
```

引数なしではCLIの使い方を表示します。初期セットアップは `init` です。コマンドはその版の `--help` を参照してください。

入口の実行にはNode.js 18以上を使います。Node.jsを入れていないBun環境では `bunx --bun https://github.com/aomona/enderpin/releases/download/v0.2.1/enderpin-0.2.1.tgz --help` のように起動できます。更新時も希望する版のURLを指定してください。npmレジストリの公開版はv0.1.3のため、`enderpin@latest` ではv0.2.1を取得できません。

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

表のファイル名に置き換えてください。特定の版に固定する場合は、`releases/latest/download/` を `releases/download/v0.2.0/` のような公開済みタグのパスに置き換えます。URL版の更新時も希望する版のURLで `bun install -g` を実行します。Bun 1.4.2では旧バイナリが残る場合を確認しています。`enderpin --version` で確認し、更新されていなければ `bun remove -g enderpin` の後に同じインストールコマンドを再実行してください。ゲームの保存データは削除しません。

このOS別パッケージには実行ファイルとJava/JNIブリッジを同梱しています。インストールにRust・JDK・Node.jsや `bun pm trust` は不要です。`enderpin` が見つからない場合は `bun pm bin -g` が示すディレクトリをPATHへ追加してください。Linuxのbubblewrapなど、ゲーム起動に必要なOS側の条件は引き続き必要です。Linuxの配布版はglibc向けで、Alpine Linux（musl）用ではありません。

## リポジトリ内のWrapperを使う {#repository-wrapper}

v0.1.5以降には `wrapper` コマンドが含まれます。v0.1.4にはこのコマンドがありませんが、生成したWrapperからv0.1.4を起動することもできます。

構成を配る側で、一度だけ生成します。format 4ではv0.2.0以降を指定してください。`init`では自動生成しません。

```sh
enderpin wrapper --version 0.2.1
git add enderpinw enderpinw.ps1 enderpin-wrapper.properties
git commit -m "Add pinned Enderpin wrapper"
```

`--version` を省略すると、生成時点の最新の安定版を選びます。GitHub APIのリリース情報と全5環境のアーカイブを取得し、GitHubのSHA-256と照合して、版・アーカイブ・実行ファイルのハッシュを設定ファイルへ固定します。APIの利用制限中やハッシュのないリリースでは生成を中止します。生成に使ったソース版と、固定される公開版は異なる場合があります。

共有先はcloneしたディレクトリから実行します。

```sh
./enderpinw sync --locked
./enderpinw run
```

WindowsではWindows PowerShell 5.1以上で実行します。実行ポリシーで禁止されている場合は、内容を確認してから、このプロセスだけ許可します。

```powershell
.\enderpinw.ps1 sync --locked
# 必要な場合のみ
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\enderpinw.ps1 sync --locked
```

macOS / Linuxでは `sh`・`curl`・`tar` と `sha256sum` または `shasum`、WindowsではPowerShellと `tar.exe` が必要です。Enderpin・Rust・Node.js・Bunは不要ですが、[ゲーム起動のOS要件](platforms.md)は引き続き適用します。Windowsはx64のみ対応します。

初回は対象環境のアーカイブを取得し、SHA-256を検証して実行ファイルだけをキャッシュします。以後も起動前にキャッシュを検証し、不一致なら停止します。`--offline` はEnderpin側の指定なので、Wrapperの初回取得にはネット接続が必要です。一度キャッシュすればWrapperの起動に通信は不要です。

キャッシュはmacOSの `~/Library/Caches/enderpin/wrapper/`、Linuxの `${XDG_CACHE_HOME:-~/.cache}/enderpin/wrapper/`、Windowsの `%LOCALAPPDATA%\enderpin\wrapper\` に保存します。必要なら `ENDERPIN_WRAPPER_CACHE` で場所を指定できます。壊れたキャッシュは、エラーに表示したディレクトリを削除して再取得してください。

更新するときだけ `enderpin wrapper --version VERSION --force` で3ファイルを再生成し、差分を確認してコミットします。通常の起動や `enderpin update` ではWrapperの固定版を変更しません。3ファイルは実行するコードとその検証値なので、Gitからの変更もレビューしてください。ゲーム権限の承認・ログイン・EULA同意は従来どおり明示的に行います。

## 最新のソース版

ソースからビルドする場合はRust 1.89以上、JDK 17以上、Cコンパイラーが必要です。

```sh
git clone https://github.com/aomona/enderpin.git
cd enderpin
cargo install --path . --locked
```

## format 4とWrapper

v0.2.0以降を使用してください。v0.1.8以前を固定したWrapperはformat 4に対応しません。既存のWrapperは`enderpin wrapper --version 0.2.1 --force`で更新し、差分を確認してコミットします。
