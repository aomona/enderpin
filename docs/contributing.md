# 開発と検証

開発ビルドにはRustに加えてJDK 17以上（`java` / `javac` / `jar`）とCコンパイラーが必要です。Java/JNIブリッジをビルドして同梱します。クロスコンパイル時は対象OSのJDKヘッダーを `ENDERPIN_TARGET_JDK` で指定します。ゲーム用のJavaは引き続き別途固定・自動取得します。

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
python3 -B tests/wrapper.py  # ローカルfixtureで取得・検証・キャッシュ・引数転送を確認

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

GitHub ActionsにはWindows・macOS・LinuxのRustチェックと実JVMのサンドボックス検証を定義しています。PRの作成・更新時と`main`へのpush時に実行し、作業ブランチへのpushでは起動しません。`dev`もPRを通して検査します。ワークフロー定義だけでは、各OSで実際に検証に成功したことを意味しません。[検証記録](verification.md) に今回実行した範囲を記載しています。

## Windowsのカーソル閉じ込めテスト

対話デスクトップ上のJava 25で次を実行できます。現在のカーソル制限範囲を再設定し、
通常実行とクライアントAppContainerでは成功、サーバーAppContainerでは拒否されることを確認します。
カーソルの移動や制限範囲の変更は行いません。ヘッドレスCIではこの実入力テストを実行せず、
通常テストで専用ウィンドウステーション上の権限・並行する起動の権限除去を検証します。

```powershell
$env:ENDERPIN_TEST_JAVA_HOME = 'C:\path\to\jdk-25'
$env:ENDERPIN_TEST_DESKTOP = '1'
cargo test --locked --test windows_cursor -- --ignored --nocapture
```

## 認証ブリッジの組込API

Rustの利用側は `enderpin::auth::bridge` から、ビルド時に埋め込まれた認証ブリッジを取得できます。

```rust
use enderpin::auth::bridge::{AGENT, BOOTSTRAP, NATIVE, ASSETS};

assert_eq!(AGENT.name, "auth-bridge.jar");
assert_eq!(BOOTSTRAP.name, "auth-bootstrap.jar");
assert!(!NATIVE.bytes.is_empty());
for asset in ASSETS {
    // asset.name: &'static str、asset.bytes: &'static [u8]
    // 保存先の検査・ファイルの配置・起動は利用側で行います。
    assert!(!asset.bytes.is_empty());
}
```

`EmbeddedAsset` はファイル名と読み取り専用バイト列だけを公開します。`ASSETS` はエージェントJAR・ブートストラップJAR・JNIライブラリの3点で、ナレーターを含みません。このAPI自体はファイルを書き込まず、資格情報の読み出し・認証仲介の開始・権限承認も行いません。エージェントはJNIライブラリと同じディレクトリにある `auth-bootstrap.jar` を読み込むため、利用側はこの配置を維持してください。

Javaクラスは `--release 17` で生成します。ビルドにはホスト側のJDK 17以上の `java` / `javac` / `jar` と、対象向けのCコンパイラーが必要です。JNIライブラリはRustのコンパイル対象OS・CPU向けであり、実行時に別環境向けを選ぶ仕組みではありません。Windowsは `auth-bridge.dll`、macOSは `libauth-bridge.dylib`、Linuxは `libauth-bridge.so` になります。

クロスコンパイルでは対象向けのCツールチェーンに加え、対象OSのJNIヘッダーを含むJDKを `ENDERPIN_TARGET_JDK` で指定します。この変数はヘッダーの選択だけに使われ、ホストで実行するJavaツールはPATHから取得します。ビルド済みのバイト列を配置するだけなら、利用時にJDKの開発ツールやCコンパイラーは不要です。対象OS・CPUの一致に加えて、既存の対応ゲーム版・Java版・authlib検査と認証の失敗時停止は利用側で引き続き必要です。

`tests/auth_bridge_assets.rs` は公開APIだけを使い、ファイル名、対象OSのネイティブ形式、JARのクラス分離・マニフェスト・Java 17バイトコードを検査します。これは実ゲーム認証や各OSでのJNI動作を証明するテストではありません。

## Bun配布パッケージ

Python 3.11以上とBunを使って、現在のOS・CPU向けのパッケージを作成・検証できます。

```sh
cargo build --release --locked
python3 scripts/package_bun.py target/release/enderpin
python3 tests/bun_install.py target/bun/enderpin-darwin-arm64.tgz
# 手元へインストールする場合
bun install -g ./target/bun/enderpin-darwin-arm64.tgz
```

Windowsでは実行ファイルを `target/release/enderpin.exe`、パッケージ名は[対応環境](platforms.md)に合わせて指定します。検証スクリプトは一時HTTPサーバーからBunで取得し、隔離したグローバル領域へインストールして、バージョン表示・ヘルプ・エラー終了・空白を含むパスでのワークスペース作成を確認します。普段のBunグローバル領域は変更しません。ゲーム起動の検証は含みません。

`.github/workflows/release.yml` は5環境でビルドとBunインストール検証を行います。手動実行ではActionsの成果物だけを作成します。`Cargo.toml` と一致する `vVERSION` タグをpushすると、全環境の成功後に `.tgz` を添付したReleaseの下書きを作成します。内容を確認して公開すると[インストール手順](installation.md)から取得できます。対応するソースは同じタグのGitHubソースアーカイブから取得できます。

## npm配布パッケージ

5環境のOS別アーカイブを同じ版で揃えてから作成します。

```sh
python3 scripts/package_npm.py --tag v0.2.0 --binaries packages
python3 tests/npm_install.py target/npm/enderpin-0.2.0.tgz
npm publish target/npm/enderpin-0.2.0.tgz --dry-run
# npmレジストリへの公開も別途行う場合のみ
npm publish target/npm/enderpin-0.2.0.tgz --access public
python3 tests/npm_install.py target/npm/enderpin-0.2.0.tgz --registry
```

バージョンはCargo.tomlから生成します。起動用JavaScript・メタデータ・README・LICENSEと、5種類の実行ファイルだけを同梱します。npm依存はありません。Releaseワークフローはビルド後に同梱版を5環境でnpm/Bun/pnpm検証し、全件成功後にアーカイブを下書きへ添付します。

手元の1環境だけで検証する場合は `python3 scripts/package_npm.py --allow-partial` を使えます。このテスト用パッケージには `private: true` が入り、npmへ公開できません。検証は標準のNode.js/npm CLI、Bun、pnpmを使い、グローバル領域とキャッシュを一時ディレクトリへ分離します。


## ドキュメントサイトを更新する

Node.js 22以上とnpmを使います。Rustのビルドとは独立しています。

```sh
npm ci
npm run docs:dev
npm run docs:build
python3 scripts/check_docs.py
npm run docs:preview
```

`docs/` のMarkdownがそのままページになります。READMEは導入の要約に留め、詳細は対応するページを更新してください。新しいページは `docs/.vitepress/config.mts` のナビゲーションへ追加します。ビルド時に内部リンクを検証します。

`docs:preview` はWranglerで実際の静的アセット配信を確認します。`docs:build` の出力先は `docs/.vitepress/dist/`。Workerスクリプト・API・DBを持たず、HTML・CSS・JavaScript・同梱フォントだけを [Workers Static Assets](https://developers.cloudflare.com/workers/static-assets/) へ配信します。存在しないページは404として返します。

Cloudflareへ公開する場合は、対象アカウントを確認してから実行します。

```sh
npx wrangler login
npx wrangler whoami
npm run docs:deploy
```

Worker名は `enderpin-docs`、設定はリポジトリ直下の `wrangler.jsonc` です。認証情報はリポジトリへ保存しません。

### デザインの参照元

MonaLauncherの `design/README.md`、`MonaLauncher_Design_Definition_v1.1.md`（1〜5・17〜18章）、`MonaLauncher_Design_Tokens_v1.1.json` を参照しています。トークンのスナップショットは `.vitepress/design-tokens.json` に保持し、色変数を `scripts/docs-theme.mjs` で生成します。

無彩色のLight/Dark、240pxサイドバー、900px未満の折り畳み、32px/16px余白、24/32の見出し、記事用の16/28本文、36px操作領域、通常要素の影なしを適用します。Geist・Noto Sans JP・Geist Monoは同梱し、フォントライセンスも公開成果物へ含めます。日本語等幅フォントはOSフォールバックです。

サイト名とGitHubへのリンクはWebサイトのヘッダーに置き、アプリ固有の起動・アカウント操作は追加しません。検索は文書本文のみを対象にします。テーマの選択はサイドバーの「表示設定」にまとめます。

VitePress 1.6.4のビルド依存Viteは、開発サーバーの既知修正を含む6.4.3に固定しています。依存更新時は `npm audit` とビルド・プレビュー・狭幅表示・検索を確認してください。VitePress側で対応する版へ更新したら、`package.json` のoverrideを見直します。
