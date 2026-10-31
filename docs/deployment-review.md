# デプロイ入力のレビューと記録

`deploy/cd.py` は `plan` と `apply` を分けます。plan はホストへの接続やvaultの復号をせず、入力のrevisionと有効な非秘密設定を記録します。apply は同じ入力を再検証し、レビューしたmanifestと一致する場合だけAnsibleを実行します。

## 版管理するものと秘密情報

`deploy/ansible/inventory.yml` を設定リポジトリへコミットします。instance配置、integration、ポート、バイナリの識別情報などは非秘密設定です。メモリ・JVM設定は `enderpin.toml`、ゲーム設定は選択ターゲットの `files/` に置きます。配布するファイルはすべてGit追跡対象でなければなりません。ignored/untrackedのローカルファイルは再現できないため拒否します。

ホスト配置を公開できない場合は、inventoryを別の保護された設定リポジトリまたは不変artifactで版管理し、controllerで取得してください。checkout外のinventoryには `--config-revision <immutable-revision> --config-sha256 <sha256-of-inventory>` をplan/apply双方で指定します。manifestにもホスト情報が含まれるため、レビューartifactの閲覧権限も設定リポジトリと同じ範囲に制限してください。

inventoryは1つの静的YAMLです。`group_vars/`・`host_vars/` の別ファイルはinventoryへ統合してください。applyにはレビュー済みのホスト変数とグループ所属を固定した一時inventoryを渡します。Jinjaのlookupや運用設定をvault経由で上書きする方法は利用できません。既存の `ENDERPIN_INVENTORY` Secretをファイルへ移行してください。秘密値を含むキーや未対応変数は拒否します。非秘密と宣言したフィールドに秘密値を埋め込まないでください。

vaultには `enderpin_forwarding_secret`、`ansible_password`、`ansible_become_password`（互換名 `ansible_ssh_pass` / `ansible_become_pass`）のみを許可します。GitHub Secretsの `ENDERPIN_VAULT` に暗号化vault、`ENDERPIN_VAULT_PASSWORD` に復号パスワードを設定します。その他のSSH鍵は信頼するcontrollerで管理します。

`ENDERPIN_SECRET_REFERENCE` はsecret storeの参照名、`ENDERPIN_SECRET_REVISION` はその不変revisionです。GitHub Secretsにはrevision取得機能がないため、このテンプレートではローテーションごとにRepository variable `ENDERPIN_SECRET_REVISION` を新しい識別子へ更新します。値や値のハッシュをrevisionに使わないでください。秘密値はGitにもmanifestにも書かず、秘密値の変更にGitへの公開は不要です。secret storeとrevisionの対応は運用者が保証します。

## バイナリの固定

各ホストの `enderpin_binary` は配布先アーキテクチャの実行ファイルです。`enderpin_binary_version` にレビューしたrelease/build識別子、`enderpin_binary_sha256` にその実行ファイルのSHA-256を設定します。圧縮アーカイブのdigestとは異なります。controller側でplan/apply時に検証し、ホスト上のステージ済みファイルも実行前に検証します。既存のcontrollerパスのファイルが差し替わっても、同じworkspace commitのまま検証を通過できません。

## 手動レビュー

専用controllerの永続ディレクトリを配布用アカウントだけが書き込める状態で用意します。生成物をcheckout内へ置くとclean checkout検証に失敗するため、外へ保存してください。

```sh
export GITHUB_SHA="$(git rev-parse HEAD)"
export ENDERPIN_SECRET_REFERENCE='vault:production/minecraft'
export ENDERPIN_SECRET_REVISION='revision-2026-11-01-01'
# vault/passwordの環境変数はapply時だけ注入
.venv/bin/python -B deploy/cd.py plan \
  --manifest /var/lib/enderpin-cd/proposed.json \
  --previous /var/lib/enderpin-cd/last.json
# proposed.json と proposed.diff、変更されたfiles/をレビュー
.venv/bin/python -B deploy/cd.py apply \
  --manifest /var/lib/enderpin-cd/proposed.json \
  --previous /var/lib/enderpin-cd/last.json \
  --record /var/lib/enderpin-cd/last.json
```

初回はpreviousファイルがなくても構いません。その場合は全体を新規入力として表示します。以後は最後に成功したmanifestとの差分になります。レビュー待ちの間にbinary、inventory、secret revision、previous recordが変わるとapplyを拒否します。失敗時には成功記録を更新しません。複数ホストの部分的な適用を巻き戻す機能ではありません。

manifest schema version 1にはsource commit、設定revisionと有効inventoryのdigest、選択ホスト/ターゲット、バイナリidentity、設定の出所、生成テンプレートのdigest、secret reference/revisionを記録します。native設定ファイルそのものやvaultの内容・digestは出力しません。ゲーム設定の詳細は記録されたsource commitの `files/` 差分でレビューします。生成設定はテンプレートと非秘密入力、secret revisionで追跡します。

Ansible/inspectorの標準出力・標準エラーは、エラーに秘密値が混ざる可能性があるため公開ログへ転送しません。失敗時はcontroller上で入力と接続設定を確認してください。vaultの一時ファイルは0600で作成し、失敗時も削除します。

## GitHub Actions

`deploy/github-actions.yml` はplan artifactをアップロードした後、`minecraft-production` Environmentを待つ2ジョブです。Environmentのrequired reviewersを設定し、レビュー担当者は `deployment-review` のmanifest/diffを確認してから適用を承認します。Environment保護を設定しないと自動でapplyへ進むため、レビュー境界には保護設定が必要です。

両ジョブは同じ信頼するcontroller storageを使います。`/var/lib/enderpin-cd` を事前作成し、複数リポジトリの場合はworkflowの `ENDERPIN_STATE` を専用パスへ変更してください。専用runnerが複数ならこの永続状態と固定バイナリを共有する構成にしてください。別workflowや手動操作も含め同時配布を避けます。適用成功後は `deployment-applied` artifactを保存します。長期監査に必要ならartifact保持期限外にも保管してください。

これらの記録は信頼するcontroller、コミットしたplaybook、固定済みゲーム入力を前提とします。OSパッケージ、controllerのSSH/PKI状態、実行時のワールドデータまで再現するsnapshotではありません。実環境でのゲーム接続、VPN、サービス再起動は別途確認してください。
