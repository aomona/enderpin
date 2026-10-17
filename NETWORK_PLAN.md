# Minecraftネットワーク構成

ホストの性能や配置は固定しません。Velocity・Paper・Fabricそれぞれを独立したEnderpinワークスペースとして同じGitリポジトリに置き、Ansible inventoryで配置先、アドレス、ポート、メモリを指定します。

- Minecraft版とFabric Loader/Paperビルド/Velocity版を個別に指定し、各ワークスペースのlockを共有します。
- OpenVPNは任意です。分散時には公開側をVPNサーバー、内部側を外向きVPNクライアントにできます。
- Velocity modern forwarding、FabricProxy-LiteとPaperの設定、バックエンドのポート制限をAnsibleで管理します。
- 指定ブランチへのpushを配布と権限適用の承認として扱います。稼働中ワークスペースは停止後にバックアップし、実際に反映した内容のfingerprintを承認します。

具体的な設定、初回構築、CD、復旧手順は[デプロイ手順](docs/deployment.md)を参照してください。テンプレートは`deploy/`にあります。実機のVPN通信、ゲーム接続、性能は別途検証が必要です。
