# Velocity・複数サーバーのデプロイ

1つのGitリポジトリにサーバー別のワークスペースを置き、Ansible inventoryで配置先を指定します。VelocityとPaper/Fabricを同じホストにも別ホストにも配置できます。OSはDebian 12以降またはUbuntu 24.04以降のsystemd環境を対象としたテンプレートです。ハードウェアやクラウド事業者は固定しません。

## リポジトリとバージョン

Enderpinのソース版をビルドし、`deploy/`をサーバー設定用リポジトリへコピーします。`deploy/example/servers/`をリポジトリ直下の`servers/`へ、同ディレクトリの`.gitignore`もコピーしてください。

```text
servers/
  proxy/enderpin.toml       # Velocity
  lobby/enderpin.toml       # Paper
  survival/enderpin.toml    # Fabric + FabricProxy-Lite
  .../enderpin.lock         # 各ディレクトリで生成してGitに追加
  .../files/server/         # 追加のMOD設定など
deploy/ansible/             # inventoryとplaybook
```

サンプルのMinecraft 1.21.1、Paper 133、Fabric Loader 0.16.10、Velocity 3.4.0-SNAPSHOT+563は形式を示す固定例です。最新版や推奨版という意味ではありません。利用するMinecraft・MOD・プラグインに合わせて変更してください。

| `server.loader` | `server.loader_version` |
| --- | --- |
| `fabric` | Fabric Loader版。例: `"0.16.10"`。省略時は安定版を解決 |
| `paper` | Minecraft版に対応するビルド番号。例: `"133"`。省略時はSTABLEチャンネルの最大ビルド |
| `velocity` | Velocity版、または`版+ビルド番号`。指定必須。例: `"3.4.0-SNAPSHOT+563"`。ビルド省略時はその版のSTABLEチャンネルから選択 |

Velocityの版は`minecraft`と独立しています。`minecraft`はパッケージの互換性判定に使用します。Paper/Velocityには`kind = "plugin"`のパッケージを指定します。Fabricには`kind = "mod"`を指定します。異なるバックエンドに異なるMinecraft版も指定できますが、プレイヤーのプロトコルやMODの互換性は別途揃える必要があります。

```sh
enderpin -C servers/proxy sync --target server
enderpin -C servers/lobby sync --target server
enderpin -C servers/survival sync --target server
git add servers
```

初回の`sync`がランタイムjar、必要なJava、MODとその依存をlockに固定します。版を変更したときも`sync`してlockをコミットします。自動更新はせず、CDは`sync --locked`を使います。`files/server/`には共有する設定だけを置き、ワールド、アカウント、VPN鍵、SSH鍵をコミットしないでください。

## 配置と認証

AnsibleコントローラーにはPython 3.11以降、OpenSSL、SSH、パスワード不要のsudoが使える配布先アカウントを準備します。CIの場合は専用の信頼できるLinux self-hosted runnerを使用します。runnerを小さなゲーム用ホストに置く必要はありません。

```sh
python3 -m venv .venv
.venv/bin/pip install -r deploy/ansible/requirements.txt
cp deploy/ansible/inventory.example.yml deploy/ansible/inventory.yml
cp deploy/ansible/vault.example.yml deploy/ansible/vault.yml
openssl rand -hex 32
.venv/bin/ansible-vault encrypt deploy/ansible/vault.yml
```

生成した64桁のhex値をvaultの`enderpin_forwarding_secret`に設定してから暗号化します。再配布ごとに生成し直さず、プロキシとバックエンドで同じ値を維持してください。Gitには暗号化したvaultだけを保存するか、CIのsecretで渡します。SSHのknown_hostsは事前に登録します。

`inventory.yml`で次を設定します。

- `minecraft.hosts.<host>.enderpin_instances`: サービス名、リポジトリ内workspace、bind address、port、memory_mib。同じホストのポートと名前は重複できません。
- `enderpin_binary`: コントローラー上の検証済みLinuxバイナリ。配布先のCPUアーキテクチャに合わせてホストごとに変更できます。ホスト上ではビルドしません。
- `enderpin_backends`と`enderpin_try`: Velocityから到達できるIPv4:portと接続順序。
- `enderpin_proxy_address`と`enderpin_backend_interface`: バックエンドへの接続を許可するプロキシの送信元と受信インターフェース。
- `enderpin_accept_eula`: Minecraft EULAに同意した場合だけ`true`にします。Velocityだけのホストでは不要です。

Ansibleが`server.properties`、`velocity.toml`、`forwarding.secret`、`config/FabricProxy-Lite.toml`、`config/paper-global.yml`、`spigot.yml`を生成します。これらを`files/common/`や`files/server/`に重ねて置くと配布を拒否します。バックエンドの追加プロパティはinstanceの`properties`辞書で指定します。接続・認証・RCON設定は配布側が最後に固定します。

Fabricワークスペースには有効な直接依存として`modrinth = "fabricproxy-lite"`を指定し、lockに含めてください。Ansibleは`hackOnlineMode = true`と共有secretを設定します。必要な構成ではinstanceに`fabric_proxy_hack_early_send` / `fabric_proxy_hack_message_chain`のbooleanを指定できます。Paperは1.19以降の設定形式を対象とし、modern forwardingを有効、legacy BungeeCordを無効にします。

同一ホストでは、バックエンドを`127.0.0.1`、プロキシの接続先も`127.0.0.1:port`にし、VPNグループを削除して`vpn.yml`を省略できます。複数ホストの場合はVPN内アドレスなどを指定します。

## OpenVPNの初回構築

`vpn_gateway`グループに1台、`vpn_clients`に内部側ホストを指定します。クライアント名と固定IPは`enderpin_vpn_clients`にも同じ名前で記載します。VPNサブネットは既存のLANと重複しないIPv4 /16〜/29を選び、最初の利用可能アドレスをサーバーに指定します。

```text
プレイヤー → 公開ホスト:25565/TCP → Velocity
                                      ↓ VPN内IP:port
                                 Paper / Fabric
内部ホスト → 公開ホスト:1194/UDP       # OpenVPNの接続開始方向
```

自宅ルーターでMinecraftのポートを開ける必要はありません。初回だけ、内部ホストにSSHで到達できるLANなどの経路をinventoryに指定して構築します。サンプルのVPN内SSHアドレスとProxyJumpはVPN構築後に使う値です。VPNがない段階でその値のまま実行すると接続できません。必要ならコントローラーをLANに置いて初回構築し、そのPKIをCIコントローラーへ安全に引き継ぎます。

```sh
cd deploy/ansible
../../.venv/bin/ansible-playbook -i inventory.yml vpn.yml --ask-vault-pass -e @vault.yml
```

CAと各ホストの鍵はコントローラーの`enderpin_pki_directory`に永続保存します。CA秘密鍵をホストへ配りません。既存の証明書は再利用し、有効期限の7日前から失敗させます。自動的な鍵の上書きや更新はしないため、証明書の更新・侵害時のCA変更は運用者が計画して行ってください。PKIはバックアップし、使い捨てrunnerやcheckout内へ置かないでください。

VPNはTUN/UDPで、LAN全体の経路やデフォルトルートを変更しません。Velocity自身がVPN経由でバックエンドへ接続するため、この用途ではDNATや全通信のVPN化は不要です。公開側OSとクラウドの既存ファイアウォールには、公開ゲームTCPポート、VPN UDPポート、管理SSHの必要な許可を設定してください。playbookは既存ファイアウォール全体を置き換えません。

バックエンド保護用のnftablesテーブルだけを管理し、loopbackと指定したプロキシのVPN内IPからの接続を許可し、それ以外のバックエンドポートをIPv4/IPv6とも拒否します。既存のUFWなどがさらに拒否する場合は、そちらにも必要な許可が必要です。VPNクライアント間の一般的なルーティングは構築しません。

## 手動配布とpushによるCD

```sh
# 設定リポジトリのコミット済みの内容から実行
cd deploy/ansible
../../.venv/bin/ansible-playbook -i inventory.yml deploy.yml \
  --ask-vault-pass -e @vault.yml -e "enderpin_revision=$(git rev-parse HEAD)"
```

`deploy/github-actions.yml`を設定リポジトリの`.github/workflows/deploy.yml`にコピーします。既定の配布ブランチは`deploy`です。専用self-hosted runnerには`enderpin-cd`ラベル、SSHの認証・known_hosts、永続PKIディレクトリ、配布用バイナリを用意します。

GitHub Secretsには`ENDERPIN_INVENTORY`（inventory全文）、`ENDERPIN_VAULT`（暗号化したvault全文）、`ENDERPIN_VAULT_PASSWORD`を設定します。VPNもCD対象にする場合はRepository variable `ENDERPIN_MANAGE_VPN=true`を設定します。不要なら省略してください。

このブランチへのpushはMOD・プラグイン・管理設定・定義されたプロセス権限の承認として扱います。workflow、playbook、inventoryもホスト管理権限を持つため、ブランチの書き込み権限と保護ルールを管理してください。別のworkflowから同じ環境への同時配布は避けます。

ホストとinstanceをinventory順に1つずつ配布します。バックエンドをプロキシより先に並べてください。各instanceで以下を実行します。

1. 使い捨てディレクトリへmanifest・lock・設定を置き、lockedダウンロードを検証します。
2. 稼働中サービスを停止し、ワールドとローカル状態をprivateな`backups/`へ保存します。
3. 共有入力を交換し、既存のローカルignoreを除き、lockと共有設定から管理対象ファイルを復元します。
4. 反映済みの権限planを取得し、そのfingerprintを承認します。ホストに残る外部フォルダ許可は自動承認しません。
5. sandbox付きsystemdサービスを起動し、TCP待ち受けとサービスの稼働を確認します。

承認後の変更は起動時に再検証します。配布前のダウンロード失敗では現在のサービスを停止しません。切り替え後に失敗した場合は停止してバックアップを残し、後続の配布も中断します。自動で古いワールドへ戻すことはありません。配布にはサービスの停止時間が発生し、複数ホスト全体の原子的な切り替えは保証しません。

`run --service`は標準入力のEOFで終了せず、SIGTERMでPaper/Fabricへ`stop`、Velocityへ`end`を送ります。終了を待つ間はsystemdがJVMを残し、180秒で強制終了します。ログは`journalctl -u enderpin-<name>`で確認できます。削除したinstanceのサービス・アカウント・ワールドを自動削除する処理はありません。廃止するサービスは明示的に停止・無効化してください。

## 復旧と検証範囲

バックアップは`<enderpin_root>/<instance>/backups/`に残り、自動削除しません。ディスク使用量を管理してください。秘密情報を含むため、instance専用のprivateディレクトリで保管します。復旧するときは対象サービスを停止し、空の退避先でtarの内容を確認してからworkspaceを戻します。Minecraft版を戻す場合のワールド互換性は自動判定しません。

Enderpinの`--offline`はEnderpinの固定済み入力の準備に適用します。Paperの初回起動時のbootstrapやプラグイン自身の通信まで無効にする指定ではありません。ゲームのネットワーク権限は通常どおりsandboxで制御します。

構文・設定生成・証明書発行・ロック処理の検証と、実ホストでの接続確認は別です。TCP待ち受け確認だけではMinecraftログイン、forwardingされたUUID/IP、MOD互換性、VPNの再接続、TPSやメモリ消費を保証しません。初回構築ではこれらを実環境で確認してください。
