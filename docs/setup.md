# 初期設定

`init`は構成・ランタイム・パッケージと、編集を始めるための標準設定を用意します。通常のゲームセッションは起動せず、ゲームの権限やEULAを自動承認しません。

```sh
enderpin init my-world --minecraft 26.3 --loader fabric --loader-version 0.19.5 --no-interactive
enderpin init my-network --target survival --type server \
  --minecraft 26.3 --loader fabric --loader-version 0.19.5 --no-interactive
```

省略した版やローダーは端末で選べます。`--mods lithium`はFabric用です。`--yes`は選択を省略する指定で、実行権限の承認には使いません。`--type proxy --loader velocity`には`--loader-version`が必要です。

認証を有効にしたクライアントでは、`init`・`sync`・`update` が起動時と共通の対応表を使い、ランタイムやパッケージの重いダウンロード前にMinecraft・Fabric・Javaの対応を検査します。Fabricの版を省略した場合はupstreamのstableを選びますが、認証アダプター未対応なら停止して明示的な固定を案内します。自動で古い版へ切り替えたり、認証を無効化したりはしません。現在の対応と実証範囲は [認証対応表](auth-compatibility.md) を参照してください。サーバーのみの準備にはこのクライアント認証の制限を適用しません。公式authlibのハッシュ検査は認証付き起動時にも行います。

## 一つのmanifestから複数環境を用意する

[設定例](configuration.md)の`enderpin.toml`を先に書き、次を実行します。

```sh
enderpin init --target all --no-interactive
```

既存manifestの内容を読み、全対象の準備と未作成の初期設定生成を行います。既存manifestに対して`--minecraft`などの作成オプションを併用するとエラーです。設定を変更するにはmanifestを編集してください。失敗した準備は同じコマンドで再実行できます。`init --target survival`なら一つだけ対象にします。

## 標準設定の生成方法

- **Vanilla・Fabric・Paperのserver.properties**: 固定されたMinecraft版の公式サーバーを、MODやプラグインを読み込まない一時領域で`--initSettings`付き実行します。サンドボックスはネットワーク・デスクトップ・ワークスペースへのアクセスを与えず、一時領域と検証済みランタイムだけを使用します。生成した`server.properties`だけを`<名前>/files/`へ取り込みます。
- **Velocity**: 固定JAR内の`default-velocity.toml`を検証・抽出して`<名前>/files/velocity.toml`へ配置します。プロキシを起動しません。
- **クライアント、MOD・プラグイン固有設定、Paper固有YAML**: 通常起動や操作が必要なものは自動生成の対象外です。ソフトが生成した設定をレビューし、共有したいものだけ`files/`へ移します。

PaperにもMinecraft標準のpropertiesを初期値として使い、Paper独自の既定値を再現したと主張しません。古いMinecraftで`--initSettings`が利用できない場合やVelocityに標準設定が同梱されていない場合は、説明付きで停止します。通常起動への自動フォールバックはありません。

初期化は最大60秒で打ち切り、子プロセスを終了して一時領域を破棄します。`eula.txt`、生成された秘密情報、ログ、ワールドは共有入力へ取り込みません。`server.properties`の初期化にはJVM実行が含まれますが、選択したMOD・プラグインやユーザーJVM引数を実行しません。

既存の`files/`や選択された共有元の設定は上書きしません。`run/`だけに同名の設定がある場合も自動取り込みせず、移行手順を表示します。生成後の編集は[通常の同期](workspace.md)で反映し、`sync`や`update`では初期値を再生成しません。

## 起動

初期設定を編集してから、対象を指定して起動します。

```sh
enderpin run --target survival --accept-eula
enderpin run --target proxy
```

実行権限は起動時に別途確認します。`--accept-eula`は内容を確認し同意した場合だけ指定します。

`init`はWrapperを自動生成しません。リポジトリで実行版を固定するには、`enderpin wrapper --version 0.2.0`を実行してください。
