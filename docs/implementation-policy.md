# 実装方針

## 目的

Jenkins の代替として、シンプルなジョブ実行サービスを Rust で構築する。

このサービスは以下を重視する。

- 構成の単純さ
- 長期運用しやすさ
- ジョブ実行状態の見通しの良さ
- Jenkins 互換を持たない明快な設計

## 前提

- 実装言語は Rust とする
- Jenkins 互換は考慮しない
- Docker を必須要件にしない
- 標準運用はオンプレミス環境への直接導入を想定する
- セキュアな構成が必要な場合は、利用者側の判断で Docker 化や VM 化を行う
- サービス側では Docker 配布や Docker 前提機能を提供しない

## 基本方針

- ワークフローは DAG として表現する
- 1 ノードは 1 タスク実行単位とする
- タスクは自由なコードではなく、制約付きの定義データとして扱う
- 成功判定は終了コードを基本とし、必要に応じて成果物チェックを追加する
- ログ、状態、再実行、キャンセルを初期段階から扱う
- プラグイン前提の設計にはしない

## 実行モデル

- ジョブはホスト環境上で実行する
- 実行方式として Docker を前提にしない
- タスクは `program` と `args` を分離して定義する
- 任意のシェル文字列をそのまま評価する設計は避ける
- 将来の拡張余地は持たせても、初期実装ではローカル実行のみを対象にする

例:

```yaml
name: sample-build
nodes:
  - id: test
    program: cargo
    args: ["test"]
    working_dir: /opt/repos/sample
    timeout_sec: 1800

  - id: build
    program: cargo
    args: ["build", "--release"]
    working_dir: /opt/repos/sample
    depends_on: ["test"]
    outputs:
      - target/release/sample
```

## セキュリティ方針

Docker による隔離を前提にしないため、実行制約を強めに設計する。

- 専用の実行ユーザーでサービスを動作させる
- ジョブごとに作業ディレクトリを明示する
- 許可されたディレクトリ配下のみで動作させる
- 実行可能なプログラムや参照先を制御できるようにする
- タイムアウト、停止、強制終了をサポートする
- 実行ログと監査情報を保存する
- 同時実行数と排他制御を持つ

注意:

- サービス自体を Docker 化しても、ジョブをホストで実行する限り完全隔離にはならない
- そのため、Docker は本サービスの必須要件として扱わない

## 機能の優先順位

MVP では以下を優先する。

1. ジョブ定義の登録と読込
2. DAG に基づく順次実行
3. ノード単位の成功 / 失敗管理
4. 実行ログ保存
5. 実行履歴表示
6. 再実行
7. キャンセル

初期段階では以下を対象外とする。

- Jenkins 互換
- GUI ベースのフローエディタ
- プラグイン機構
- Docker 前提の実行
- 分散エージェント

## 推奨技術構成

- Web/API: `axum`
- 非同期実行: `tokio`
- シリアライズ: `serde`
- 定義フォーマット: YAML または JSON
- DB: PostgreSQL
- DB アクセス: `sqlx`
- ログ配信: WebSocket または SSE
- プロセス実行: `tokio::process::Command`

## 設計上の注意点

- 実行基盤と UI の責務を分離する
- 状態遷移を先に定義してから実装する
- 成果物の存在だけで成功判定しない
- 実行環境差異の影響を受ける前提で、設定と運用条件を明示する
- ユーザーに自由度を与えすぎず、制約によって保守性を確保する

## 想定する状態遷移

ノード状態は最低限以下を持つ。

- `pending`
- `running`
- `success`
- `failed`
- `skipped`
- `canceled`

ジョブ全体も同様に、進行中・完了・失敗・中止を明確に持つ。

## 次の具体化対象

次に詰めるべき内容は以下。

- ジョブ定義スキーマ
- 状態遷移仕様
- 実行制約の詳細
- API の最小セット
- DB スキーマ
- MVP のディレクトリ構成

## 引き続き具体化する箇所

以下は、実装着手前に具体化しておくべき項目である。

### 1. ジョブ定義スキーマ

MVP で採用する正式なジョブ定義スキーマを以下の通り確定する。

#### 1-1. 正式フォーマット

- 正式フォーマットは YAML とする
- 内部的には Rust 構造体へデシリアライズして扱う
- JSON は将来的に API 入力で受ける可能性はあるが、MVP の定義ファイル形式にはしない

#### 1-2. 目的

ジョブ定義は、自由なスクリプト記述ではなく、DAG 実行に必要な最小限の構造化データとして扱う。

- 1 ジョブは複数ノードで構成する
- 1 ノードは 1 プロセス実行単位とする
- ノードは依存関係によって実行順を決める
- ノードの成功は終了コードを基本とし、必要に応じて成果物存在確認を加える

#### 1-3. ルート構造

```yaml
version: 1
id: sample-build
name: Sample Build
description: Sample project build workflow
defaults:
  working_dir: /opt/repos/sample
  timeout_sec: 1800
  retry: 0
  env: {}
nodes:
  - id: test
    name: Run tests
    program: cargo
    args: ["test"]

  - id: build
    name: Build release
    program: cargo
    args: ["build", "--release"]
    depends_on: ["test"]
    outputs:
      - path: target/release/sample
        required: true
```

#### 1-4. ルート項目

| 項目 | 型 | 必須 | 説明 |
|---|---|---|---|
| `version` | integer | 必須 | スキーマバージョン。MVP では `1` 固定 |
| `id` | string | 必須 | ジョブ定義の識別子 |
| `name` | string | 必須 | 表示名 |
| `description` | string | 任意 | 説明 |
| `defaults` | object | 任意 | ノード共通の既定値 |
| `nodes` | array | 必須 | ノード定義の配列。1 件以上必須 |

#### 1-5. defaults 項目

`defaults` は各ノードに共通する既定値であり、ノード個別設定が優先される。

| 項目 | 型 | 必須 | 説明 |
|---|---|---|---|
| `working_dir` | string | 任意 | 既定の作業ディレクトリ |
| `timeout_sec` | integer | 任意 | 既定のタイムアウト秒数 |
| `retry` | integer | 任意 | 既定の自動リトライ回数 |
| `env` | object | 任意 | 既定の環境変数マップ |

既定値のルール:

- `defaults` は省略可能
- `defaults.env` はキーと値の文字列マップとする
- ノード側に同名キーがある場合はノード側を優先する
- `working_dir` はノード側に未指定なら `defaults.working_dir` を継承する
- `timeout_sec` と `retry` も同様に継承する

#### 1-6. node 項目

各ノードは以下の構造を持つ。

| 項目 | 型 | 必須 | 説明 |
|---|---|---|---|
| `id` | string | 必須 | ノード識別子。ジョブ内で一意 |
| `name` | string | 任意 | 表示名。未指定時は `id` を使用 |
| `program` | string | 必須 | 実行プログラム名またはパス |
| `args` | array[string] | 任意 | 引数配列。未指定時は空配列 |
| `working_dir` | string | 任意 | 実行時の作業ディレクトリ |
| `depends_on` | array[string] | 任意 | 依存ノード ID 一覧 |
| `env` | object | 任意 | ノード固有の環境変数 |
| `timeout_sec` | integer | 任意 | ノード単位のタイムアウト秒数 |
| `retry` | integer | 任意 | ノード単位の自動リトライ回数 |
| `outputs` | array[object] | 任意 | 成果物定義 |

MVP では以下の項目は入れない。

- `when`
- 条件分岐
- ループ
- ノード内スクリプト
- 実行ユーザー切替
- 任意シェル文

#### 1-7. outputs 項目

`outputs` は成果物の存在確認を行うための定義とする。

| 項目 | 型 | 必須 | 説明 |
|---|---|---|---|
| `path` | string | 必須 | 成果物パス |
| `required` | boolean | 任意 | 必須成果物か。未指定時は `true` |

MVP の成果物チェック仕様:

- `required: true` の成果物が 1 つでも存在しなければノード失敗とする
- サイズ、ハッシュ、更新時刻の検証は MVP では行わない
- パスは `working_dir` 基準の相対パスを推奨する

#### 1-8. 命名規則

識別子には次の制約を設ける。

- `id` は英小文字、数字、ハイフン、アンダースコアのみ許可する
- 先頭文字は英字とする
- ジョブ `id` はシステム全体で一意とする
- ノード `id` はジョブ内で一意とする

推奨正規表現:

```text
^[a-z][a-z0-9_-]*$
```

#### 1-9. バリデーションルール

定義読込時に以下を検証する。

- `version` は `1` であること
- `id`、`name`、`nodes` が存在すること
- ノード数が 1 以上であること
- ノード `id` に重複がないこと
- `depends_on` の参照先が存在すること
- 依存関係が循環していないこと
- `timeout_sec` は 1 以上の整数であること
- `retry` は 0 以上の整数であること
- `program` が空文字でないこと
- `args` は文字列配列であること
- `env` は文字列から文字列へのマップであること
- `outputs.path` が空文字でないこと

#### 1-10. 上書き優先順位

各項目の優先順位は以下とする。

1. ノード個別設定
2. `defaults`
3. システム既定値

システム既定値の初期案:

- `args`: `[]`
- `depends_on`: `[]`
- `env`: `{}`
- `retry`: `0`
- `timeout_sec`: `1800`

`working_dir` はシステム既定値を持たず、ノードまたは `defaults` のどちらかで必須とする。

#### 1-11. 成功判定

MVP におけるノード成功条件は次の通りとする。

- プロセス終了コードが `0`
- `required` な成果物が定義されている場合、それらがすべて存在する

失敗条件:

- 終了コードが `0` 以外
- タイムアウト
- 起動失敗
- 必須成果物の欠落

#### 1-12. サンプル定義

```yaml
version: 1
id: rust-sample-build
name: Rust Sample Build
description: Build and test a Rust project
defaults:
  working_dir: /srv/repos/rust-sample
  timeout_sec: 1800
  retry: 0
  env:
    RUST_BACKTRACE: "1"
nodes:
  - id: fmt
    name: Check format
    program: cargo
    args: ["fmt", "--check"]

  - id: test
    name: Run test suite
    program: cargo
    args: ["test"]
    depends_on: ["fmt"]

  - id: build
    name: Build release binary
    program: cargo
    args: ["build", "--release"]
    depends_on: ["test"]
    outputs:
      - path: target/release/rust-sample
        required: true
```

#### 1-13. Rust 構造体イメージ

```rust
#[derive(Debug, Deserialize)]
struct JobDefinition {
    version: u32,
    id: String,
    name: String,
    description: Option<String>,
    defaults: Option<JobDefaults>,
    nodes: Vec<NodeDefinition>,
}

#[derive(Debug, Deserialize)]
struct JobDefaults {
    working_dir: Option<String>,
    timeout_sec: Option<u64>,
    retry: Option<u32>,
    env: Option<std::collections::BTreeMap<String, String>>,
}

#[derive(Debug, Deserialize)]
struct NodeDefinition {
    id: String,
    name: Option<String>,
    program: String,
    args: Option<Vec<String>>,
    working_dir: Option<String>,
    depends_on: Option<Vec<String>>,
    env: Option<std::collections::BTreeMap<String, String>>,
    timeout_sec: Option<u64>,
    retry: Option<u32>,
    outputs: Option<Vec<NodeOutput>>,
}

#[derive(Debug, Deserialize)]
struct NodeOutput {
    path: String,
    required: Option<bool>,
}
```

#### 1-14. この時点で確定したこと

- 定義ファイル形式は YAML
- ジョブは `version`, `id`, `name`, `nodes` を必須とする
- ノードは `id`, `program` を必須とする
- `working_dir` はノードか `defaults` のどちらかで必須とする
- 成功判定は終了コードと成果物存在確認の組み合わせとする
- 任意シェル文字列実行はサポートしない

### 2. 状態遷移仕様

MVP で採用する状態遷移仕様を以下の通り確定する。

#### 2-1. 基本方針

- 状態はジョブ全体とノード単位で分けて管理する
- ノードの状態変化を集約してジョブ全体の状態を決定する
- 依存ノードが失敗した後続ノードは `skipped` とする
- キャンセルは要求状態と完了状態を分けて扱う
- タイムアウトは失敗の一種ではなく独立状態として扱う

#### 2-2. ジョブ状態

ジョブ実行の状態は以下とする。

| 状態 | 説明 |
|---|---|
| `created` | 実行レコード作成直後。まだキュー投入されていない |
| `queued` | 実行待ち。まだノード実行は始まっていない |
| `running` | 1 つ以上のノードが実行中、または実行可能ノードのスケジューリング中 |
| `cancel_requested` | ユーザーがキャンセル要求を出し、停止処理中 |
| `success` | すべての必要ノードが成功して完了 |
| `failed` | 1 つ以上のノードが失敗してジョブ全体を失敗として終了 |
| `timed_out` | ジョブ全体の制御上、実行継続不可と判断してタイムアウト終了 |
| `canceled` | キャンセル要求により停止完了 |

#### 2-3. ノード状態

各ノード実行の状態は以下とする。

| 状態 | 説明 |
|---|---|
| `pending` | 実行対象だが、依存解決またはキュー投入待ち |
| `queued` | 実行可能になり、実行待ちキューに載っている |
| `running` | プロセス起動済みで実行中 |
| `success` | 終了コード 0 かつ成果物検証も成功 |
| `failed` | 起動失敗、非 0 終了、成果物不備などで失敗 |
| `timed_out` | ノード単位タイムアウトで停止した |
| `cancel_requested` | 停止要求を受け、終了待ち |
| `canceled` | 停止要求により終了した |
| `skipped` | 依存失敗または上位判断で実行対象から外れた |

#### 2-4. ノード状態遷移

ノードは以下の遷移のみ許可する。

```text
pending -> queued
pending -> skipped
queued -> running
queued -> cancel_requested
running -> success
running -> failed
running -> timed_out
running -> cancel_requested
cancel_requested -> canceled
cancel_requested -> failed
cancel_requested -> timed_out
```

補足:

- `pending -> skipped` は依存ノード失敗時に発生する
- `queued -> cancel_requested` は未実行のままキャンセル要求を受けた場合に発生する
- `cancel_requested -> failed` は停止処理中に異常終了した場合に使う
- 一度終端状態に入ったノードは同一実行内では再遷移しない

終端状態:

- `success`
- `failed`
- `timed_out`
- `canceled`
- `skipped`

#### 2-5. ジョブ状態遷移

ジョブは以下の遷移のみ許可する。

```text
created -> queued
queued -> running
queued -> cancel_requested
running -> cancel_requested
running -> success
running -> failed
running -> timed_out
cancel_requested -> canceled
cancel_requested -> failed
cancel_requested -> timed_out
```

終端状態:

- `success`
- `failed`
- `timed_out`
- `canceled`

#### 2-6. ジョブ終了判定

ジョブ全体の終了状態は、全ノードの終端状態に基づき次のように決定する。

- すべてのノードが `success` または `skipped` で終了し、少なくとも 1 ノードが `success` の場合、ジョブは `success`
- 1 ノードでも `failed` があれば、ジョブは `failed`
- 1 ノードでも `timed_out` があれば、ジョブは `timed_out`
- キャンセル要求後に実行中および待機中ノードがすべて停止し、少なくとも 1 ノードが `canceled` なら、ジョブは `canceled`

優先順位:

1. `timed_out`
2. `failed`
3. `canceled`
4. `success`

#### 2-7. 依存関係に基づく遷移ルール

- 依存先ノードがすべて `success` になった時点で、後続ノードは `pending` から `queued` に遷移できる
- 依存先に 1 つでも `failed` または `timed_out` がある場合、後続ノードは `skipped` にする
- 依存先に `canceled` がある場合も、後続ノードは `skipped` にする
- `skipped` ノードを依存先に持つ後続は、依存の評価結果として実行不能とし `skipped` にする

MVP では、失敗後も独立ノードを継続実行するかどうかを選べるようにはしない。
1 ノードが `failed` または `timed_out` になった時点で、新たなノード起動は行わない。

#### 2-8. キャンセル仕様

キャンセル要求時のルールは以下とする。

- ジョブに対してキャンセル要求が来たら、ジョブ状態を `cancel_requested` にする
- `running` ノードは `cancel_requested` に遷移させ、停止シグナルを送る
- `queued` ノードは `cancel_requested` に遷移させた後、そのまま `canceled` にする
- `pending` ノードは `skipped` にする
- 停止完了後、ジョブ状態を `canceled` にする

キャンセル要求後の詳細:

- 正常停止できたノードは `canceled`
- 停止中にタイムアウトしたノードは `timed_out`
- 停止処理中に異常終了したノードは `failed`

#### 2-9. タイムアウト仕様

タイムアウトはノード単位で判定する。

- `timeout_sec` 経過時点で対象ノードに停止要求を送る
- 停止猶予時間経過後も終了しない場合は強制終了する
- 強制終了されたノードの状態は `timed_out` とする
- ノードが `timed_out` になった場合、ジョブ全体は最終的に `timed_out` とする

MVP ではジョブ全体の別個の wall-clock タイムアウトは持たず、ノードタイムアウトの集約結果で扱う。

#### 2-10. 再実行時の扱い

再実行は既存実行の状態遷移ではなく、新しい実行レコードとして扱う。

- 再実行時は新しい `job_run` を作成する
- 以前の実行状態は変更しない
- ノード状態も新しい `node_run` として生成する
- 親実行との関連は `rerun_of` のような参照で持つ

MVP では以下のみをサポートする。

- ジョブ全体の再実行

MVP では以下は対象外とする。

- 失敗ノードのみ再実行
- 特定ノードからの再開
- 成功済みノード結果の使い回し

#### 2-11. 実行開始時の初期状態

ジョブ実行開始時は次のように初期化する。

- ジョブ状態は `created`
- 実行キュー投入時に `queued`
- スケジューラが処理を開始した時点で `running`
- 依存のないノードは `pending` で生成し、実行可能判定後に `queued` へ遷移する
- 依存を持つノードも初期状態は `pending` とする

#### 2-12. 状態遷移イベント

監査および UI 更新のため、各遷移時にイベントを残す。

- 対象種別: `job` または `node`
- 対象 ID
- 遷移前状態
- 遷移後状態
- 発生時刻
- 原因種別
- 補足メッセージ

原因種別の初期案:

- `scheduled`
- `dependency_satisfied`
- `dependency_failed`
- `process_started`
- `process_exited`
- `process_failed`
- `timeout`
- `cancel_requested`
- `cancel_completed`
- `artifact_missing`

#### 2-13. この時点で確定したこと

- 依存失敗時の後続ノードは `skipped` とする
- タイムアウトは独立状態 `timed_out` として扱う
- キャンセルは `cancel_requested` を経由して `canceled` に至る
- 再実行は新規実行レコードを作る
- 1 ノード失敗またはタイムアウト以降、新たなノード起動は行わない

### 3. 実行制約の詳細

MVP で採用する実行制約仕様を以下の通り確定する。

#### 3-1. 基本方針

- ジョブはホスト上で直接実行する
- Docker や VM による隔離は前提にしない
- その代わり、実行可能範囲を `working_dir` と実行形式で制限する
- 任意シェル実行はサポートしない
- 同一 `working_dir` の競合制御はシステムでは行わず、利用者責任とする

#### 3-2. working_dir 制約

`working_dir` は実行制御の中心とする。

- すべてのノードは最終的に有効な `working_dir` を持たなければならない
- `working_dir` は事前に設定された許可ルート配下でなければならない
- `working_dir` の未指定は不可とする
  - ノード個別指定
  - または `defaults.working_dir`
  - のどちらかが必須
- ノードは `working_dir` を後続ノードと共有してよい
- ノードは必要に応じて個別の `working_dir` で上書きしてよい

許可ルートの例:

```text
/srv/repos
/srv/workspaces
```

#### 3-3. パス制約

ファイル操作対象は `working_dir` 配下に限定する。

- 上位ディレクトリへのアクセスを基本禁止とする
- `..` を使った上位参照を禁止する
- シンボリックリンクを解決した結果が `working_dir` 外になる場合は拒否する
- 絶対パス指定は、正規化後に `working_dir` 配下である場合のみ許可する

この制約は少なくとも以下に適用する。

- 成果物パス
- ジョブ定義上の対象ファイルパス
- 実行前にシステムが検査できるパス引数

注意:

- 一般コマンドの引数すべてについて完全な意味解析は行わない
- そのため、MVP では後述の通り実行プログラム自体も制限する

#### 3-4. 実行可能プログラム制約

MVP では、実行可能プログラムを allowlist 方式で管理する。

- 定義に記載できる `program` は、システム設定で許可されたものに限る
- `program` はコマンド名または許可済み絶対パスとする
- 未許可プログラムは実行前バリデーションで拒否する

初期方針:

- `bash`, `sh`, `zsh` のようなシェル起動は許可しない
- `python -c`, `node -e` のようなインラインコード実行前提の使い方は許可しない
- 明確な用途がある実行プログラムのみを登録する

許可候補の例:

- `cargo`
- `npm`
- `pnpm`
- `make`
- `cmake`
- `ls`
- `cat`

ただし `ls` や `cat` も無制限に許可するのではなく、`working_dir` 配下を対象とする前提で運用する。

#### 3-5. 実行形式の制約

- 実行は必ず `program + args` 形式で行う
- `shell = true` 相当の実行は行わない
- 1 ノードは 1 プロセス実行のみとする
- ノード内で複数コマンドを連結する機能は持たない

禁止対象の例:

- `bash -c "npm test && npm run build"`
- `sh -c "..."`
- `cmd /c ...`
- `powershell -Command ...`

#### 3-6. 環境変数制約

環境変数は明示的に管理する。

- 実行時環境変数は、システム既定値とジョブ定義の `env` をマージして作る
- ノード `env` は `defaults.env` を上書きする
- システム側で禁止対象の環境変数キーを持てるようにする
- 値が文字列でないものは許可しない

MVP の方針:

- 最低限、環境変数キー名の形式検証を行う
- 危険度の高いキーは設定で拒否可能にする
- システム予約キーの上書き可否はシステム設定で制御する

#### 3-7. タイムアウト制約

- すべてのノードは `timeout_sec` を持つ
- `timeout_sec` 未指定時は既定値を適用する
- タイムアウト時は停止要求を送り、猶予後に強制終了する
- 強制終了されたノードは `timed_out` とする

システム既定値の初期案:

- ノード既定タイムアウト: `1800` 秒
- 停止猶予: `10` 秒

#### 3-8. 同時実行と排他

MVP の制約は以下とする。

- システム全体の最大同時実行数は設定可能にする
- ノードの起動数はこの上限を超えてはならない
- 同一 `working_dir` の排他は行わない
- 同一ジョブ定義の複数実行も禁止しない

明示事項:

- 同一 `working_dir` を共有する複数実行の衝突防止は利用者責任とする
- システムは `working_dir` の競合を検知しない
- 成果物上書きや途中ファイル競合の可能性がある

#### 3-9. 停止制約

停止処理は次の順で行う。

1. 対象プロセスに通常停止シグナルを送る
2. 停止猶予時間待つ
3. 停止していなければ強制終了する

MVP では OS 差異は内部実装で吸収するが、意味としては以下で統一する。

- 通常停止要求
- 強制終了

#### 3-10. 実行ユーザー制約

- サービスは専用の実行ユーザーで動作させることを前提とする
- ノード単位で実行ユーザーを切り替える機能は MVP では持たない
- 実行権限はサービス起動ユーザーの権限に従う

利用者に委ねる事項:

- サービスをどの権限で動かすか
- どのディレクトリにアクセス権を与えるか
- ホスト上の実行可能プログラムをどう整備するか

#### 3-11. 読取系コマンドの扱い

`ls` や `cat` のような読取系コマンドは利用可能とするが、次の前提を置く。

- 対象は `working_dir` 配下に限定する
- 上位パスや外部パスへの参照は許可しない
- 実装上明確に検査できないケースは、許可プログラム側で制限する

つまり、許可の意味は「任意パス参照の許可」ではなく、「`working_dir` 配下を対象とした読取用途の許可」である。

#### 3-12. 利用者責任として扱う範囲

MVP では以下は利用者責任とする。

- 同一 `working_dir` を使う複数実行の競合
- ジョブ定義の内容妥当性
- ホスト上ツールのインストール
- 実行ユーザーの権限設計
- 読取系コマンドで何を読ませるかという運用判断

#### 3-13. この時点で確定したこと

- `working_dir` は必須
- `working_dir` は許可ルート配下でなければならない
- 上位パスへの操作は基本禁止
- 実行は `program + args` に限定する
- 任意シェル実行はサポートしない
- 実行可能プログラムは allowlist 管理とする
- 同一 `working_dir` の排他は行わない
- 競合は利用者責任とする

### 4. ログ仕様

ログを保存・配信・参照する単位を定義する。

- ジョブ単位ログ
- ノード単位ログ
- 標準出力
- 標準エラー出力
- 開始時刻
- 終了時刻
- 終了コード

決めるべき論点:

- DB 保存とファイル保存のどちらを主とするか
- ログのリアルタイム配信方式
- ログの保持期間
- 大容量ログの切り詰めやローテーション方針

### 5. API の最小セット

MVP で採用する API 最小セットを以下の通り確定する。

#### 5-1. 基本方針

- API と UI は同一バイナリで提供する
- MVP のジョブ定義管理は API 登録ではなく、定義ファイルの読込結果を DB に反映する方式を基本とする
- API は閲覧、実行、キャンセル、ログ参照に絞る
- 状態更新のリアルタイム配信は SSE を採用する
- MVP では認証を必須機能にしない

#### 5-2. エンドポイント一覧

| メソッド | パス | 用途 |
|---|---|---|
| `GET` | `/api/jobs` | ジョブ定義一覧取得 |
| `GET` | `/api/jobs/:job_id` | ジョブ定義詳細取得 |
| `POST` | `/api/jobs/:job_id/runs` | ジョブ実行開始 |
| `GET` | `/api/runs` | 実行履歴一覧取得 |
| `GET` | `/api/runs/:run_id` | 実行詳細取得 |
| `POST` | `/api/runs/:run_id/cancel` | 実行キャンセル要求 |
| `POST` | `/api/runs/:run_id/rerun` | ジョブ全体再実行 |
| `GET` | `/api/runs/:run_id/logs` | 実行ログ取得 |
| `GET` | `/api/runs/:run_id/events` | 実行イベント取得 |
| `GET` | `/api/runs/:run_id/stream` | 実行状態ストリーム取得 |

#### 5-3. ジョブ定義一覧取得

`GET /api/jobs`

返却項目の初期案:

- `job_id`
- `name`
- `description`
- `definition_path`
- `definition_hash`
- `enabled`
- `updated_at`

用途:

- ジョブ一覧画面
- 実行対象選択

#### 5-4. ジョブ定義詳細取得

`GET /api/jobs/:job_id`

返却項目の初期案:

- ジョブ定義の基本情報
- 現在有効な定義ファイルパス
- 現在の定義ハッシュ
- ノード一覧

補足:

- ノード一覧は定義ファイルを都度読み込むか、別途キャッシュする
- MVP では DB にノード定義そのものは保存しない

#### 5-5. ジョブ実行開始

`POST /api/jobs/:job_id/runs`

用途:

- 指定ジョブの新規実行

リクエスト初期案:

```json
{
  "trigger_type": "manual",
  "triggered_by": "admin"
}
```

返却項目:

- `run_id`
- `status`
- `queued_at`

#### 5-6. 実行履歴一覧取得

`GET /api/runs`

クエリ初期案:

- `job_id`
- `status`
- `limit`
- `offset`

返却項目:

- `run_id`
- `job_id`
- `job_name`
- `status`
- `trigger_type`
- `triggered_by`
- `queued_at`
- `started_at`
- `finished_at`

#### 5-7. 実行詳細取得

`GET /api/runs/:run_id`

返却項目:

- ジョブ実行情報
- ノード実行一覧
- 失敗理由
- 定義ハッシュ
- 再実行元情報

用途:

- 実行詳細画面
- ノード状態表示

#### 5-8. 実行キャンセル要求

`POST /api/runs/:run_id/cancel`

用途:

- 実行中または待機中ジョブにキャンセル要求を出す

返却項目:

- `run_id`
- `status`
- `cancel_requested_at`

#### 5-9. ジョブ全体再実行

`POST /api/runs/:run_id/rerun`

用途:

- 対象実行を元にジョブ全体を新規再実行する

返却項目:

- `run_id`
- `rerun_of_run_id`
- `status`

#### 5-10. 実行ログ取得

`GET /api/runs/:run_id/logs`

クエリ初期案:

- `node_id`
- `stream`
- `after_sequence`
- `limit`

返却項目:

- `sequence`
- `node_id`
- `stream`
- `content`
- `occurred_at`

#### 5-11. 実行イベント取得

`GET /api/runs/:run_id/events`

返却項目:

- `scope`
- `event_type`
- `from_status`
- `to_status`
- `message`
- `occurred_at`

用途:

- タイムライン表示
- 状態変化の監査

#### 5-12. 実行状態ストリーム取得

`GET /api/runs/:run_id/stream`

方式:

- Server-Sent Events

配信対象:

- ジョブ状態変更
- ノード状態変更
- ログ追加通知
- 終了通知

SSE を採用する理由:

- 双方向通信が不要
- 実装が比較的単純
- 実行状況監視用途に十分

#### 5-13. MVP で含めない API

MVP では以下を提供しない。

- ジョブ定義の API 登録
- ジョブ定義の API 更新
- 部分再実行 API
- ノード単位再実行 API
- 認証 / 認可 API
- Webhook API

#### 5-14. この時点で確定したこと

- API と UI は同一バイナリで提供する
- 定義管理はファイルベースを基本とする
- リアルタイム配信は SSE を採用する
- MVP では認証を必須にしない
- API は閲覧、実行、再実行、キャンセル、ログ参照に絞る

### 6. DB スキーマ

MVP で採用する DB スキーマ方針を以下の通り確定する。

#### 6-1. 基本方針

- DB は SQLite を採用する
- DB はジョブ定義の版管理を行わない
- ジョブ定義の履歴管理は `git` 等の外部手段に委ねる
- DB は現在利用する定義の参照情報と実行履歴を保存する
- 成果物本体は保存せず、メタデータのみ保持する
- ログは MVP では SQLite に保存する

#### 6-2. 永続化する対象

MVP では以下を永続化する。

- ジョブ定義の参照情報
- ジョブ実行履歴
- ノード実行履歴
- 状態遷移イベント
- 実行ログ
- 成果物検証結果

#### 6-3. テーブル一覧

MVP のテーブルは以下とする。

- `job_definitions`
- `job_runs`
- `node_runs`
- `run_events`
- `run_logs`
- `run_artifacts`

`job_definition_revisions` は作成しない。

#### 6-4. job_definitions

ジョブ定義そのものの版は持たず、現在有効な定義の参照情報を持つ。

想定カラム:

| カラム | 型のイメージ | 必須 | 説明 |
|---|---|---|---|
| `id` | INTEGER | 必須 | 内部識別子 |
| `job_id` | TEXT | 必須 | ジョブ定義の論理 ID |
| `name` | TEXT | 必須 | 表示名 |
| `description` | TEXT | 任意 | 説明 |
| `definition_path` | TEXT | 必須 | 定義ファイルパス |
| `definition_hash` | TEXT | 必須 | 現在定義のハッシュ |
| `enabled` | BOOLEAN | 必須 | 実行可能か |
| `created_at` | TIMESTAMP | 必須 | レコード作成日時 |
| `updated_at` | TIMESTAMP | 必須 | 最終更新日時 |

制約:

- `job_id` は一意
- `definition_path` は運用上の参照情報であり、版管理用途ではない
- `definition_hash` は定義内容の識別用であり、履歴テーブルは持たない

#### 6-5. job_runs

ジョブ単位の実行履歴を保持する。

想定カラム:

| カラム | 型のイメージ | 必須 | 説明 |
|---|---|---|---|
| `id` | INTEGER | 必須 | 実行 ID |
| `job_definition_id` | FK | 必須 | 対象ジョブ定義 |
| `job_id` | TEXT | 必須 | 実行時点のジョブ ID 冗長保持 |
| `job_name` | TEXT | 必須 | 実行時点のジョブ名冗長保持 |
| `status` | TEXT | 必須 | ジョブ状態 |
| `trigger_type` | TEXT | 必須 | 手動実行などの起動種別 |
| `triggered_by` | TEXT | 任意 | 実行要求者 |
| `definition_path` | TEXT | 必須 | 実行時に使った定義ファイルパス |
| `definition_hash` | TEXT | 必須 | 実行時に使った定義ハッシュ |
| `working_dir` | TEXT | 必須 | ジョブ既定の作業ディレクトリ |
| `queued_at` | TIMESTAMP | 任意 | キュー投入時刻 |
| `started_at` | TIMESTAMP | 任意 | 実行開始時刻 |
| `finished_at` | TIMESTAMP | 任意 | 実行終了時刻 |
| `cancel_requested_at` | TIMESTAMP | 任意 | キャンセル要求時刻 |
| `rerun_of_job_run_id` | FK | 任意 | 再実行元ジョブ実行 ID |
| `failure_reason` | TEXT | 任意 | 失敗理由概要 |
| `created_at` | TIMESTAMP | 必須 | レコード作成日時 |

補足:

- `definition_hash` を保持することで、どの定義内容で実行したか追跡できる
- 再実行は既存実行の更新ではなく、新規レコード作成で扱う

#### 6-6. node_runs

ノード単位の実行結果を保持する。

想定カラム:

| カラム | 型のイメージ | 必須 | 説明 |
|---|---|---|---|
| `id` | INTEGER | 必須 | ノード実行 ID |
| `job_run_id` | FK | 必須 | 所属するジョブ実行 |
| `node_id` | TEXT | 必須 | ノード識別子 |
| `node_name` | TEXT | 任意 | 実行時点のノード名 |
| `status` | TEXT | 必須 | ノード状態 |
| `program` | TEXT | 必須 | 実行プログラム |
| `args_json` | TEXT | 必須 | 実行引数 JSON |
| `working_dir` | TEXT | 必須 | 実行時作業ディレクトリ |
| `env_json` | TEXT | 任意 | 実行時環境変数 JSON |
| `timeout_sec` | INTEGER | 必須 | 実行時タイムアウト |
| `retry_count` | INTEGER | 必須 | 実際に行ったリトライ回数 |
| `exit_code` | INTEGER | 任意 | プロセス終了コード |
| `started_at` | TIMESTAMP | 任意 | ノード開始時刻 |
| `finished_at` | TIMESTAMP | 任意 | ノード終了時刻 |
| `cancel_requested_at` | TIMESTAMP | 任意 | キャンセル要求時刻 |
| `failure_reason` | TEXT | 任意 | 失敗理由概要 |
| `created_at` | TIMESTAMP | 必須 | レコード作成日時 |

制約:

- `job_run_id` と `node_id` の組み合わせは一意
- `args_json` と `env_json` は実行時スナップショットとして保持する

#### 6-7. run_events

状態遷移や重要イベントを時系列で保持する。

想定カラム:

| カラム | 型のイメージ | 必須 | 説明 |
|---|---|---|---|
| `id` | INTEGER | 必須 | イベント ID |
| `job_run_id` | FK | 必須 | 対象ジョブ実行 |
| `node_run_id` | FK | 任意 | 対象ノード実行 |
| `scope` | TEXT | 必須 | `job` または `node` |
| `event_type` | TEXT | 必須 | イベント種別 |
| `from_status` | TEXT | 任意 | 遷移前状態 |
| `to_status` | TEXT | 任意 | 遷移後状態 |
| `message` | TEXT | 任意 | 補足メッセージ |
| `occurred_at` | TIMESTAMP | 必須 | 発生時刻 |

用途:

- UI のタイムライン表示
- 監査ログ
- 障害解析

#### 6-8. run_logs

ログ本文を保持する。

想定カラム:

| カラム | 型のイメージ | 必須 | 説明 |
|---|---|---|---|
| `id` | INTEGER | 必須 | ログ ID |
| `job_run_id` | FK | 必須 | 対象ジョブ実行 |
| `node_run_id` | FK | 任意 | 対象ノード実行 |
| `stream` | TEXT | 必須 | `stdout` / `stderr` / `system` |
| `sequence` | BIGINT | 必須 | 表示順序 |
| `content` | TEXT | 必須 | ログ本文 |
| `occurred_at` | TIMESTAMP | 必須 | 生成時刻 |

MVP 方針:

- ログは行単位またはチャンク単位で保持する
- 表示順序を保つため `sequence` を持つ
- 大容量最適化は後回しとする

#### 6-9. run_artifacts

成果物の検証結果を保持する。

想定カラム:

| カラム | 型のイメージ | 必須 | 説明 |
|---|---|---|---|
| `id` | INTEGER | 必須 | 成果物レコード ID |
| `job_run_id` | FK | 必須 | 対象ジョブ実行 |
| `node_run_id` | FK | 必須 | 対象ノード実行 |
| `path` | TEXT | 必須 | 定義上の成果物パス |
| `resolved_path` | TEXT | 必須 | 実行時解決後パス |
| `required` | BOOLEAN | 必須 | 必須成果物か |
| `exists_flag` | BOOLEAN | 必須 | 存在確認結果 |
| `size_bytes` | BIGINT | 任意 | 存在時サイズ |
| `checked_at` | TIMESTAMP | 必須 | 検査時刻 |

MVP ではファイル本体は保存しない。

#### 6-10. 保存しないもの

MVP では以下は DB に保存しない。

- ジョブ定義の版履歴
- 成果物本体
- 分散エージェント情報
- ノードごとの中間キャッシュ
- システムメトリクスの長期保存

#### 6-11. インデックス方針

最低限、以下にインデックスを持つ。

- `job_definitions(job_id)`
- `job_runs(job_definition_id, created_at desc)`
- `job_runs(status, created_at desc)`
- `node_runs(job_run_id)`
- `node_runs(job_run_id, node_id)`
- `run_events(job_run_id, occurred_at)`
- `run_logs(job_run_id, sequence)`
- `run_logs(node_run_id, sequence)`
- `run_artifacts(node_run_id)`

#### 6-12. 参照関係

基本的な参照関係は以下。

- `job_definitions` 1 : N `job_runs`
- `job_runs` 1 : N `node_runs`
- `job_runs` 1 : N `run_events`
- `node_runs` 1 : N `run_events`
- `job_runs` 1 : N `run_logs`
- `node_runs` 1 : N `run_logs`
- `node_runs` 1 : N `run_artifacts`

#### 6-13. この時点で確定したこと

- DB は SQLite を採用する
- ジョブ定義の版管理は DB で行わない
- 定義履歴は `git` 等に委ねる
- 実行時点の定義識別には `definition_hash` を使う
- 実行履歴は `job_runs` と `node_runs` に分ける
- ログは MVP では SQLite に保存する
- 成果物はメタデータのみ保存する

#### 6-14. SQLite DDL

MVP の初期 DDL は [schema.sql](/mnt/c/Users/pkaichi/workspace/postjen/postjen_proj/db/schema.sql) とする。

### 7. 成果物の扱い

成果物を成功判定や後続処理にどう使うかを決める。

- 存在確認のみ行うか
- サイズやハッシュ確認を行うか
- ノード間で成果物パスを受け渡すか
- 保存先を固定するか

決めるべき論点:

- 成果物不備を失敗とする条件
- 成果物削除のタイミング
- 保持期限
- UI 上での表示対象

### 8. 再実行ポリシー

失敗時や手動操作時の再実行方針を定義する。

- ジョブ全体の再実行
- 失敗ノードのみ再実行
- 指定ノードからの再開
- 自動リトライ

決めるべき論点:

- 依存先成功結果を再利用するか
- リトライごとのログを分離するか
- 再実行前に成果物を掃除するか

### 9. UI の最小要件

MVP の UI は監視と操作に限定する。

- ジョブ定義一覧
- 実行ボタン
- 実行履歴一覧
- 実行詳細
- ノードごとの状態表示
- ログ表示
- キャンセル操作

決めるべき論点:

- 初期 UI をサーバレンダリングにするか
- SPA にするか
- ログ更新のリアルタイム性をどこまで求めるか

### 10. 運用前提

導入先ごとの差異を吸収するため、運用条件を明文化する。

- 対応 OS
- 必須ツール
- ディレクトリ構成
- 実行ユーザー権限
- バックアップ対象
- 障害時の復旧手順

決めるべき論点:

- Linux 専用にするか
- Windows を対象に含めるか
- PostgreSQL を必須にするか
- 単一ノード前提で始めるか

## 推奨する具体化の順序

実装前の整理は以下の順序がよい。

1. ジョブ定義スキーマ
2. 状態遷移仕様
3. 実行制約
4. DB スキーマ
5. API 最小セット
6. ログ仕様
7. 再実行ポリシー
8. UI 最小要件

## 実装着手の判断基準

以下が確定した時点で、MVP の実装を開始してよい。

- ノード定義の必須項目
- 成功 / 失敗 / スキップ / キャンセルの状態遷移
- 実行可能範囲の制約
- 永続化対象
- 最初に公開する API
- MVP で扱う UI の範囲

## Rust 実装構成

### ディレクトリ構成

MVP の実装は Rust workspace 構成で進める。

```text
.
├── Cargo.toml
├── db/
│   └── schema.sql
├── docs/
│   └── implementation-policy.md
└── crates/
    └── postjen-server/
        ├── Cargo.toml
        └── src/
            ├── config.rs
            ├── db.rs
            ├── http.rs
            └── main.rs
```

方針:

- 当面は単一バイナリ crate とする
- 既存の責務分割は `src/` 内モジュールで表現する
- 将来、必要になれば `domain` や `runner` を別 crate に切り出す

### crate 選定

MVP で採用する crate は以下とする。

| crate | 用途 |
|---|---|
| `axum` | HTTP API |
| `tokio` | 非同期ランタイム |
| `sqlx` | SQLite アクセス |
| `serde` | シリアライズ / デシリアライズ |
| `serde_json` | JSON 入出力 |
| `anyhow` | アプリケーション層のエラー伝播 |
| `tracing` | ログ出力 |
| `tracing-subscriber` | ログ初期化 |
| `tower-http` | HTTP トレース |

採用理由:

- `axum` と `tokio` で API と SSE 実装が素直
- `sqlx` は SQLite 対応が安定しており、後で DB を差し替える余地も残せる
- `serde` 系で YAML / JSON / DB 入出力の橋渡しがしやすい
- `tracing` 系で実行ログとアプリログの基盤を揃えやすい

### 現時点で採用しない crate

MVP では以下はまだ入れない。

- `clap`
- `uuid`
- `sea-orm`
- `diesel`
- `tonic`
- `reqwest`

理由:

- CLI 機能は後回し
- ID は SQLite の整数主キーで足りる
- ORM よりまずは SQL を明示したい
- gRPC や外部連携は MVP 対象外

### 雛形の現在地

- サーバ起動エントリポイントを用意
- SQLite 初期化処理を用意
- 主要 API ルートを雛形として配置
- 一部 API は `501 Not Implemented` で stub 化

注意:

- この環境では `cargo` が未導入のため、ビルド確認は未実施
