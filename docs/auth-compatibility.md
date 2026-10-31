# 認証アダプターの対応と検証

2026-11-01にmacOS 27.0.1 / Apple Siliconで確認。下表のMinecraftとVanilla・Fabric Loader **0.19.5 / 0.20.1**を許可します。認証付きクライアントでは、対応表にないMinecraft・Loader・Javaの組合せを準備前に拒否します。サーバー、明示的に認証を無効にしたクライアント、`run --demo`には、この認証アダプターの制限を適用しません。デモ起動でもロックとファイルの検証は行い、保存済みの認証設定や権限承認は変更しません。

対応は版番号だけで判定しません。Minecraftの公式メタデータが宣言するJava、authlibのファイル名とSHA-256、Fabric Loaderの公式JARのSHA-256、Javaクラスの必要なメソッド形状を検証します。authlibとLoaderの実ファイル検査は認証付き起動時、メソッド形状の検査はクラス定義時にも行い、不一致は停止します。実トークン・秘密鍵をゲームに渡すフォールバックや認証の自動無効化はありません。

## 確認した組合せ

「vendor smoke」は各版の公式ゲーム/authlib JARについて、静的な変換検査に加え、Vanillaと両Fabric LoaderのKnotクラスローダー上で参加要求・属性取得・証明書取得・秘密鍵非エクスポート・署名IPCを実行した結果です。IPC応答は合成であり、この列だけでは実アカウントの認証成功を意味しません。Knot smokeはクラス読み込みの検証で、通常のFabric初期化やMOD動作を保証するものではありません。

| Minecraft | Java major | authlib | vendor smoke（3構成） | 実ゲームでの認証付き参加 |
| --- | --- | --- | --- | --- |
| 1.20.1 | 17 | 4.0.43 | 成功 | Fabric 0.19.5 / 0.20.1 成功 |
| 1.20.2 | 17 | 5.0.47 | 成功 | 未実施 |
| 1.20.4 | 17 | 6.0.52 | 成功 | 未実施 |
| 1.20.6 | 21 | 6.0.54 | 成功 | 未実施 |
| 1.21 | 21 | 6.0.54 | 成功 | 未実施 |
| 1.21.1 | 21 | 6.0.54 | 成功 | 未実施 |
| 1.21.4 | 21 | 6.0.57 | 成功 | Fabric 0.19.5 / 0.20.1 成功 |
| 1.21.5 | 21 | 6.0.58 | 成功 | 未実施 |
| 1.21.8 | 21 | 6.0.58 | 成功 | 未実施 |
| 1.21.11 | 21 | 7.0.61 | 成功 | 未実施 |
| 26.1 | 25 | 7.0.63 | 成功 | 未実施 |
| 26.2 | 25 | 9.0.75 | 成功 | 未実施 |
| 26.3 | 25 | 10.0.77 | 成功 | Fabric 0.19.5 / 0.20.1 成功 |

公式メタデータ・JARの検査と39構成のsmokeにはTemurin 17.0.21+9 / 21.0.12.1+1 / 25.0.5+8を使用しました。メタデータのJava majorと同じJVMで実行しています。署名IPCの合成応答は暗号署名の正しさの検証ではありません。別途、既存のRust/JNIサンドボックスプローブをJava 17・21・25で実行し、テスト専用鍵の署名検証・別ハンドルの拒否・ネットワーク拒否を確認しました。Java 17では除外される5テストすべてを実行し、ファイル・ネットワークの隔離も確認しました。

実ゲームの参加検証にはTemurin 17.0.21+9 / 21.0.13+9 / 25.0.5+8を使用しました。表の実参加欄にない組合せは、今回の実ゲーム検証の対象外です。

## 互換性の変化点

- **1.20.1 / authlib 4**: 参加処理は`MinecraftClient.post`を通らず、`YggdrasilAuthenticationService.makeRequest`を呼びます。専用の`joinServer(GameProfile, String, String)`変換で、サーバーハッシュだけを共通ホストbrokerへ渡します。旧経路へのフォールバックはありません。
- **authlib 5〜7**: 参加要求がrecordになり、共通`MinecraftClient`の経路を使います。属性レスポンスの既存スキーマを維持します。
- **authlib 9〜10**: friends/chatの属性を要求する既存スキーマを維持します。authlib 10では`yggdrasil`から`services`へクラスが移動しています。
- **ゲームの難読化名**: 公式client mappingsからCrypt・デスクトップ・Realms・ファイルドロップの名前を確認し、RustとJavaが同じ[組込対応表](https://github.com/aomona/enderpin/blob/main/java/auth-bridge/auth-adapters.properties)を使用します。26.3のデスクトップ処理はBlaze3Dへ移動しています。
- **1.21系のASM**: ゲームが同梱するASMとFabricのASMを両方載せると、Knotの重複クラス検査で起動できません。共通ランタイム処理でASMのモジュールごとにFabric側を優先し、クライアント・サーバーの両方を修正しました。
- **Fabric**: 0.19.5 / 0.20.1のKnot経由でもagentがクラス定義を変換できることを確認しました。完全一致リストはこの2版に限定し、未知の将来版を無条件には許可しません。
- **macOS / Java 17**: エージェント起動時のiconvに必要なOS文字コード表`/usr/share/i18n`の読取だけを基礎ポリシーに追加しました。ホームディレクトリや任意のMachサービスの許可は追加していません。

## 再実行と限界

```sh
cargo build --locked
python3 tests/auth_compatibility.py \
  --java-home /path/to/jdk17 --java-home /path/to/jdk21 --java-home /path/to/jdk25
ENDERPIN_TEST_JAVA_HOME=/path/to/jdk17 cargo test --locked \
  native_bridge_signs_without_exporting_keys_and_enforces_denial -- --ignored
python3 tests/live_client.py /path/to/locked/workspace \
  --cache-dir /path/to/cache --accept-eula --use-account
```

vendor smokeはPOSIXのsocketpairを使用し、ネットワークから公式アーティファクトをハッシュ照合して取得します。実ゲームテストは保存済みアカウントとデスクトップを使い、使い捨てのlocalhostサーバーを`online-mode=true`・`enforce-secure-profile=true`・`white-list=false`で起動します。接続成功後はゲームとサーバーを停止し、秘密鍵キャッシュが残らないことを確認します。チャットは送信しません。macOSで明示キャッシュを指定したテストはAPFSの独立した複製でアセットを用意し、通常の起動時ハッシュ検証も通します。

1.19系以前、表にない中間リリース・snapshot、Fabricの他版は未検証のため拒否します。未対応が本質的な非互換を意味するわけではありません。公式JAR、ゲーム側の全変換対象、要求Java、Loader経由の経路を確認してから追加してください。Windows/Linuxの実ゲーム、任意のMODとの組合せ、署名付きチャット送信、Realms参加はこの検証に含めません。

一次資料: [Minecraft公式version manifest](https://piston-meta.mojang.com/mc/game/version_manifest_v2.json)、[Fabric公式Loader metadata](https://meta.fabricmc.net/v2/versions/loader)、[Fabric 0.20.1 KnotClassDelegate](https://github.com/FabricMC/fabric-loader/blob/0.20.1/src/main/java/net/fabricmc/loader/impl/launch/knot/KnotClassDelegate.java)。各JARのURL・SHA-1はversion metadata、authlibの固定SHA-256とゲーム側クラス名は組込対応表に記録しています。
