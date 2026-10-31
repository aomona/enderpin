# 解決済み設定の JSON API

```sh
enderpin -C my-network config show --resolved --target survival --json
enderpin -C my-network config show --resolved --json # 全ターゲット
```

`config show` は `--resolved --json` を必須とする読み取り専用の検査です。`--target` は宣言済みの名前または `all`（既定）です。名前が `server` の環境は必要ありません。継承・配列置換・グループの競合は、`sync` と同じ Rust の `Manifest::target` で解決します。

ネットワーク接続、キャッシュの作成、Workspace ロック取得、トランザクション復旧、権限承認、ゲーム起動は行いません。入力ファイルの選択・競合確認には `sync` と同じファイル選択処理を使います。ネイティブ設定の内容を読み込み比較しますが、内容やそのハッシュは出力しません。通常のアクセスに伴うファイルシステムの atime 更新は対象外です。

## Schema version 1

成功時は stdout に JSON 文書を一つ出力し、終了コードは 0、stderr は空です。

```json
{
  "schema_version": 1,
  "manifest_format": 4,
  "targets": {
    "survival": {
      "type": "server",
      "side": "server",
      "runtime": {"minecraft": "1.21.1", "loader": "fabric", "loader_version": null},
      "launch": {"memory_mib": 2048, "jvm_args": [], "permissions": {}},
      "profile": null,
      "sources": {},
      "groups": [],
      "file_groups": [],
      "file_inputs": ["survival/files"],
      "files": {},
      "packages": {},
      "enabled": [],
      "disabled": [],
      "lock": {
        "packages": {"status": "missing", "expected_fingerprint": "<sha512>", "fingerprint": null, "identities": []},
        "runtime": {"status": "missing", "expected_fingerprint": "<sha512>", "identity": null}
      },
      "diagnostics": [
        {"code": "packages_lock_missing", "severity": "warning"},
        {"code": "runtime_lock_missing", "severity": "warning"},
        {"code": "package_compatibility_unverified", "severity": "warning"}
      ]
    }
  }
}
```

上記は形を示す例です。実際の `permissions` と `sources` には以下のフィールドが入ります。v1 の既存フィールドの型・意味は維持します。追加フィールドと診断コードは追加可能です。消費側は未知のフィールドを無視し、未対応の `schema_version` は拒否してください。Manifest/lock の `format` と JSON の `schema_version` は別のバージョンです。

| フィールド | 型と意味 |
| --- | --- |
| `targets` | 選択した名前をキーとするオブジェクト。`all` では全宣言を含む |
| `type` / `side` | `client\|server\|proxy` / `client\|server`。proxy の side は server |
| `runtime` | `minecraft`, `loader` は文字列。`loader_version` は宣言された固定版または null |
| `launch.memory_mib` | 解決済み最大ヒープの整数。未指定は 2048 |
| `launch.jvm_args` | 配列置換後の引数配列。`=` 以降はすべて `[REDACTED]` |
| `launch.permissions` | target の権限または既定値。`files` は `game`（`read-write\|read-only`）, `read_only`（設定と同じ列挙配列）。ほかは [権限設定](configuration.md) と同じキー・値 |
| `profile` | 使用したプロファイル名、または null |
| `sources` | `type`, `minecraft`, `loader`, `loader_version`, `memory_mib`, `groups`, `file_groups`, `enabled`, `disabled`, `permissions`, `jvm_args` の出所。値は `targets.<name>`, `profiles.<name>`, `manifest`, `default` |
| `groups` / `file_groups` | 解決後のパッケージ／ファイルグループ名配列。空配列への置換も反映 |
| `file_inputs` | ワークスペース相対ディレクトリの配列。選択した共有グループの順に並び、最後に `<target>/files`。存在しないディレクトリも含む |
| `files` | インストール先の相対パス → 選択された入力元の相対パス。target ファイルは共有ファイルを上書き。同名の共有ファイルが異なる内容ならエラー |
| `packages` | alias → `{declaration, sources, direct}`。宣言は全グループと profile/target のマージ後。`sources` は出所の文字列配列（同一宣言の複数グループも記録）。`direct` は profile/target の直接宣言であることを示す |
| `packages.*.declaration` | `kind`, `prerelease` と、存在すれば `modrinth`, `url`, `version`, `optional`, `filename`。Manifest の Package と同じ型。URL は全体を `[REDACTED]` に置換 |
| `enabled` / `disabled` | 解決後の選択指定。`packages` は宣言一覧なので無効な宣言も含む |
| `lock.packages` | `status`: `current\|stale\|missing`、現在の宣言の `expected_fingerprint`、保存済み `fingerprint`（なければ null）、`identities` 配列 |
| `lock.packages.identities[]` | 保存済みパッケージの `key`, `name`, `kind`, `project`（nullable）, `version`（nullable）, `sha512`, `size`, `filename`, `roots`, `required`, `default_sides`。URL・自由形式の版表示は含まない |
| `lock.runtime` | `status`, `expected_fingerprint`, `identity`。identity は未保存なら null、それ以外は `fingerprint`, `minecraft`, `loader`, `loader_version`, `java_major` |
| `diagnostics[]` | `code` と `severity`（`warning\|error`）。パッケージ単位では `package` に alias を含む |

`packages` は依存解決結果ではありません。Modrinth の対応 side、依存関係、バージョン互換性にはオンライン情報が必要です。`lock.packages.status == current` の identities が保存された選択結果です。`stale` の identities は参考情報で、配置すべきパッケージ一覧には使えません。ローカル `.enderpinignore`、起動時 CLI オプション、外部フォルダ許可、認証状態、実行中プロセスはこの API の対象外です。

`launch` は Manifest と既定値からの有効設定です。デプロイ環境の `--memory` 等の上書きや生成済みネイティブ設定を含みません。これらの所有関係を比較する消費側は `sources`, `files`, `lock` を参照し、上書きを別に表示してください。出力をシェルコマンドとして実行したり、伏せた引数を起動に渡してはいけません。

## 診断とエラー

| 診断コード | 意味 |
| --- | --- |
| `packages_lock_missing` / `runtime_lock_missing` | 対象のロックが未保存 |
| `packages_lock_stale` / `runtime_lock_stale` | Rust core の fingerprint と不一致 |
| `package_compatibility_unverified` | 現在の宣言の依存・side・バージョン互換性をこのオフライン検査では確定できない |
| `incompatible_package_kind` | core と同じ runtime/kind 制約に違反。直接宣言または明示 enabled は error、未選択の共有宣言は warning |
| `unsupported_runtime` | Manifest では表現可能だが現行 runtime resolver が未対応（NeoForge） |

診断は成功文書の一部です。消費側は用途に応じて拒否します。Ansible は error 診断、古いロック、不足ロックを拒否します。`current` は fingerprint の一致であり、署名検証、ダウンロード済みの証明、実ゲームの互換性の保証ではありません。

検査が成立しない場合は stdout は空、終了コードは 1、stderr は次の JSON 文書一つです。元のパーサーエラー、入力行、秘密値は出力しません。

```json
{"schema_version":1,"error":{"code":"invalid_manifest","message":"Manifest schema or target configuration is invalid"}}
```

安定した `error.code` は `workspace_unreadable`, `manifest_unreadable`, `manifest_missing`, `invalid_manifest`, `unknown_target`, `lock_unreadable`, `invalid_lock`, `invalid_file_inputs` です。`invalid_manifest` は構文、未知のキー、未定義グループ、グループ競合、target/runtime 制約などを含みます。`message` は人向けで分岐条件にしないでください。引数自体の間違いや `--help` は通常の CLI 使用法メッセージです。

## 秘密と一貫性

認証ストア・Vault・ゲームデータ・ローカル権限は読みません。選択されたネイティブ入力に秘密が含まれる場合も、内容はファイル競合確認にのみ使います。入力ファイルの内容とハッシュ、ダウンロード URL、JVM の `=` 以降の値を出力しません。target/package 名、パス、JVM オプション名などの識別子は公開メタデータとして扱うため、これらに秘密を埋め込まないでください。

読み取り専用のため排他的 Workspace ロックは取りません。同時書き換え中の原子的なスナップショットは保証しません。デプロイでは固定したコミットから検査し、その入力を staging して `sync --locked` で再検証してください。
