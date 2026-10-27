# 旧形式からの移行

format 4は名前付きターゲットと`<名前>/files・run`の配置を使用します。format 3以前を読み込むと手順を示して停止し、ワールドや設定を自動で移動しません。v0.1.8以前を固定したWrapperもformat 4を解釈できません。v0.2.0以降への更新が必要です。

## format 3 → 4

1. ゲームを正常終了し、manifest・lock・`files/`・`run/`・`.enderpin/`を含む環境全体をバックアップします。
2. 新しい作業ディレクトリに`format = 4`のmanifestを作ります。`[client]`は`[targets.client]`と`type = "client"`、`[server]`は`[targets.<名前>]`と`type = "server"`にします。Velocityは`type = "proxy"`です。パッケージの子テーブルも同様に移します。
3. `[common.<MOD名>]`は`[groups.common.packages.<MOD名>]`へ移し、必要なターゲットだけに`groups = ["common"]`を指定します。複数manifestを統合する場合は、各Minecraft版・ローダー・パッケージを確認して共通化します。
4. 旧`files/client/`・`files/server/`を新しい`<名前>/files/`へコピーします。旧`files/common/`は`shared/common/files/`へコピーし、必要な対象に`file_groups = ["common"]`を指定します。パッケージ共有とファイル共有は独立しています。
5. 旧lockのファイルを保管し、旧環境の実際のloader版・直接パッケージ版をmanifestへ固定してから、新ディレクトリで`enderpin sync --target all`します。これは新しいlockを生成するため、推移依存やJavaを含む版・ハッシュの差分を旧lockと比較してください。数字のformatだけを書き換えて同じ解決結果とみなすことはできません。
6. 旧`run/<旧名>/`のワールド、プレイヤーデータ、必要なローカル設定を新`<名前>/run/`へコピーします。新しく同期されたMOD・ランタイムを旧ファイルで丸ごと上書きしないでください。共有管理に入れる既存設定は`files/`へコピーし、未管理ファイルと衝突しないよう退避します。
7. `permissions show --target <名前>`で新しい実行計画を確認し、対象ごとに承認します。旧承認をコピーして再利用しません。EULAの同意、秘密情報、外部フォルダもホストごとに確認します。
8. ワールドとプレイヤーデータが正しいことを確認してから運用を切り替えます。検証が終わるまで旧環境とバックアップを保持します。

## ターゲットの改名・削除

manifestから名前を消しても、そのディレクトリとlockの旧エントリを自動削除しません。改名は新しい名前の対象を作る操作として扱います。停止・バックアップした上で必要なデータを明示的に移し、新しい名前で同期・再承認してください。

## 権限設定の新形式へ移行する

旧`[client.sandbox]`、MOD単位の権限宣言、古い`game_write`等はサポートしません。`[targets.<名前>.permissions]`と`permissions.files`でゲームプロセス全体の設定を指定します。詳細は[設定](configuration.md)と[サンドボックス](sandbox.md)を参照してください。

## CLIの変更 {#cli-changes}

`--target`は固定のclient/serverではなくmanifestの名前を選びます。`init --target survival --type server`で名前付き環境を作り、既存manifestは`init --target all`で初期化します。以前のformat 3 CLIを使う場合は、対応する旧バージョンのドキュメントを参照してください。
