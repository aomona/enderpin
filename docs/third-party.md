# 再利用したコード

EnderpinはGPL-3.0-onlyで公開します。全文は [LICENSE](../LICENSE) を参照してください。

以下は同じ作者の [MonaLauncher](https://github.com/aomona/MonaLauncher) のGPLv3実装を移植したものです。
移植元はdevブランチの `0e2f2f79ed8b37cb1d3033a5a5e7d53772750dd3`（2026-09-14参照）です。

- `src/auth/broker/`: 認証RPC、チャット署名、Unix/Windows IPCとそのテスト。
- `src/narrator/`: bounded narrator protocolとOS別の音声処理。
- `java/auth-bridge/`, `java/narrator-bridge/` とそれぞれのsmoke test: MinecraftのJava側アダプター。

Enderpinの起動・セッション・パス・承認管理に接続する変更を加えています。
テスト用のチャット鍵は移植元と同じ公開フィクスチャで、実アカウントの秘密鍵ではありません。
