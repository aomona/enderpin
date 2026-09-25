# 対応環境と制限

`enderpin permissions` で、ゲーム全体の通信・書き込み・認証・外部フォルダの許可を対話的に編集できます。`permissions approve` または初回の `launch` で一覧を確認して承認します。権限変更やMODの追加・更新時は再確認し、変更がなければ確認を省略します。[設定例・自動化・OSごとの制約](sandbox.md) を参照してください。

- macOSはSeatbelt、Linuxはbubblewrapとseccomp、WindowsはAppContainerとJob Objectを使います。通常起動への自動切り替えはありません。
- 対象のゲームディレクトリと専用一時ディレクトリを書込可能にし、共有Java・ライブラリ・キャッシュを読取専用にします。クライアントのassetsは検証して対象別キャッシュへ複製し、スキンの書き込み許可も分離します。
- 通信は既定で許可します。`launch --no-network` はゲームのIP通信を拒否します。ポートごとの制御やOSファイアウォールの変更は行いません。
- `--offline` は固定済みファイルとキャッシュだけで準備します。認証付きクライアントのセッション更新と承認済みの認証仲介には別途ホスト側のネットワークを使います。アカウントなしの公式デモは `launch --demo --offline --no-network` で試せます（事前に `prepare` が必要）。
- Linuxにはbubblewrapと利用可能なユーザー名前空間が必要です。デスクトップ起動はローカルX11またはWayland、必要に応じてPulseAudioとGPUを使います。
- MOD用の実行環境は現代のFabricプロファイルを対象とし、Minecraft 1.21.1で実測しています。`quick` はVanillaまたはFabricのセットアップに対応します。未対応の旧式native形式やOSバージョン条件は理由を示して停止します。1.21.1のLinux ARM64クライアントは公式LWJGL nativeが適合しないため停止します（サーバーは対応）。
- Windows・Linuxの実JVMによる制限と認証ブリッジは検証済みですが、両OSのMinecraft画面・認証付きゲーム参加は未検証です。Windowsのloopback例外は自動追加しません。AppContainerの対象別プロファイルとファイル権限設定は起動後も残ります。
