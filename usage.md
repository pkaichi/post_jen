# Usage

## 前提

このプロジェクトを実行するには、以下が必要です。

- `cargo`
- `rustc`

通常は `rustup` 経由で Rust ツールチェインを導入する。

## 環境変数

| 変数 | デフォルト | 説明 |
|------|-----------|------|
| `POSTJEN_BIND_ADDR` | `127.0.0.1:3000` | リッスンアドレス |
| `POSTJEN_DATABASE_URL` | `sqlite:postjen.db` | SQLite パス |
| `POSTJEN_ARTIFACTS_DIR` | `artifacts` | 成果物の保存先ディレクトリ |
| `POSTJEN_SECRET_KEY` | なし | シークレット暗号化キー（32 バイト hex 文字列） |

例:

```bash
export POSTJEN_BIND_ADDR=0.0.0.0:3000
export POSTJEN_DATABASE_URL=sqlite:postjen.db
export POSTJEN_ARTIFACTS_DIR=/var/postjen/artifacts
export POSTJEN_SECRET_KEY=$(python3 -c "import os; print(os.urandom(32).hex())")
```

## 起動

### 単体起動

```bash
cargo run -p postjen-server
```

起動すると以下が自動的に行われる。

- SQLite スキーマの自動初期化（`db/schema.sql` ベース）
- ビルトインローカルエージェントの自動登録
- 成果物ディレクトリの作成

この状態でジョブの登録・実行・監視がすべて行える。

### リモート接続付き起動

別のマシンで起動している postjen-server にリモートエージェントとして接続する場合は `--connect-to` を指定する。

```bash
cargo run -p postjen-server -- \
  --connect-to http://コントローラーのアドレス:3000 \
  --agent-name linux-builder \
  --agent-labels linux,builder
```

この場合、自身もサーバとして動作しつつ、コントローラーのリモートエージェントとしてもタスクを受け付ける。

| オプション | 必須 | 説明 |
|-----------|------|------|
| `--connect-to` | 任意 | 接続先コントローラーの URL |
| `--agent-name` | 任意 | エージェント名。`target.agent` で指定する際に使用する。デフォルト: `remote-agent` |
| `--agent-labels` | 任意 | カンマ区切りのラベル。`target.labels` でマッチに使用する |
| `--poll-interval` | 任意 | タスクポーリング間隔（秒）。デフォルト: 2 |
| `--heartbeat-interval` | 任意 | ハートビート送信間隔（秒）。デフォルト: 15 |

### 構成例: 複数マシンでの分散実行

```text
マシンA (コントローラー)          マシンB (Linux ビルド)           マシンC (Windows ビルド)
cargo run -p postjen-server      cargo run -p postjen-server \   cargo run -p postjen-server \
                                   --connect-to http://A:3000 \    --connect-to http://A:3000 \
                                   --agent-name linux-builder \    --agent-name windows-builder \
                                   --agent-labels linux              --agent-labels windows
```

- マシン A: コントローラー。ジョブ定義を登録し、実行を管理する
- マシン B, C: 自身もサーバだが、マシン A のエージェントとしてタスクを受け取る
- ジョブ定義の `target` でどのマシンで実行するかを制御する
- 接続は B,C → A への Pull 型。A のポートのみ開放すればよい

## 基本フロー

1. ジョブ定義 YAML を用意する
2. `POST /api/jobs` でジョブ定義を登録する
3. `POST /api/jobs/:job_id/runs` で実行を作成する
4. `GET /api/runs/:run_id` `logs` `events` で状態を確認する

## サンプル定義

リポジトリには以下のサンプルを用意している。

- [sample-hello.yaml](examples/jobs/sample-hello.yaml)
  - 1 ノード成功の最小サンプル
  - ファイル出力と成果物確認を行う
- [sample-dag-success.yaml](examples/jobs/sample-dag-success.yaml)
  - 依存関係付き 2 ノード成功サンプル
  - `depends_on` による順次実行を確認できる
- [sample-failure.yaml](examples/jobs/sample-failure.yaml)
  - 失敗ノードと後続 `skipped` の確認用サンプル
- [sample-timeout.yaml](examples/jobs/sample-timeout.yaml)
  - ノードタイムアウト遷移の確認用サンプル
- [sample-remote.yaml](examples/jobs/sample-remote.yaml)
  - `target.labels` でリモート実行するノードと、ローカル実行する後続ノードの組み合わせ

## API 利用例

### ヘルスチェック

```bash
curl http://127.0.0.1:3000/api/health
```

期待例:

```json
{"status":"ok"}
```

### ジョブ一覧取得

```bash
curl http://127.0.0.1:3000/api/jobs
```

### ジョブ定義登録

```bash
curl -X POST http://127.0.0.1:3000/api/jobs \
  -H "Content-Type: application/json" \
  -d '{"definition_path":"examples/jobs/sample-hello.yaml","enabled":true}'
```

### ジョブ詳細取得

```bash
curl http://127.0.0.1:3000/api/jobs/sample-hello
```

### ジョブ実行開始

```bash
curl -X POST http://127.0.0.1:3000/api/jobs/sample-hello/runs \
  -H "Content-Type: application/json" \
  -d '{"trigger_type":"manual","triggered_by":"local-user"}'
```

期待例:

```json
{"run_id":1,"status":"queued","queued_at":"2026-03-14 12:00:00"}
```

### 実行履歴一覧取得

```bash
curl "http://127.0.0.1:3000/api/runs?limit=20&offset=0"
```

### 実行詳細取得

```bash
curl http://127.0.0.1:3000/api/runs/1
```

### 実行ログ取得

```bash
curl "http://127.0.0.1:3000/api/runs/1/logs?limit=100"
```

### 実行イベント取得

```bash
curl http://127.0.0.1:3000/api/runs/1/events
```

## ビルドパラメータ

ジョブ定義に `params` を定義し、実行時にパラメータを渡すことができる。パラメータはノードの環境変数として注入される。

### ジョブ定義

```yaml
version: 1
id: my-build
name: Parameterized Build
params:
  - name: BRANCH
    default: "main"
  - name: VERSION
    required: true
defaults:
  working_dir: /opt/repos/sample
  timeout_sec: 300
nodes:
  - id: build
    program: bash
    args: ["-c", "git checkout $BRANCH && make build VERSION=$VERSION"]
```

| 項目 | 型 | 必須 | 説明 |
|------|-----|------|------|
| `name` | string | 必須 | パラメータ名。ノードの環境変数名として使用される |
| `default` | string | 任意 | デフォルト値。未指定かつ required でない場合は注入されない |
| `required` | bool | 任意 | true の場合、実行時に値の指定が必須。デフォルト: false |

### 実行時のパラメータ指定

```bash
curl -X POST http://127.0.0.1:3000/api/jobs/my-build/runs \
  -H "Content-Type: application/json" \
  -d '{"trigger_type":"manual","params":{"BRANCH":"develop","VERSION":"1.2.3"}}'
```

- `required: true` のパラメータが未指定の場合、400 エラーになる
- 定義にないパラメータを渡した場合も 400 エラーになる
- `default` が設定されていて実行時に未指定の場合、デフォルト値が使われる
- パラメータはノードの環境変数にマージされる（ノードの `env` に同名がある場合、ノードの値が優先）

## ノード並列実行

DAG の依存関係に基づき、独立したノードは同時に実行される。

### 動作

- 依存関係のないノードはすべて同時にエージェントに割り当てられる
- いずれかのノードが完了すると、新たに依存が解決されたノードが割り当てられる
- 1 ノードでも失敗すると、新たなノード起動は停止し、未実行ノードは `skipped` になる

### 例

```yaml
nodes:
  - id: test-unit
    program: make
    args: ["test-unit"]

  - id: test-integration
    program: make
    args: ["test-integration"]

  - id: build
    program: make
    args: ["build"]
    depends_on: ["test-unit", "test-integration"]
```

この定義では `test-unit` と `test-integration` が同時に実行され、両方が成功した後に `build` が実行される。

## シークレット管理

パスワードやAPIキー等のシークレットを暗号化して保存し、実行時にノードの環境変数として注入できる。

### 前提

シークレット機能を使用するには `POSTJEN_SECRET_KEY` 環境変数の設定が必要。

```bash
# 32 バイトのランダムな hex 文字列を生成
export POSTJEN_SECRET_KEY=$(python3 -c "import os; print(os.urandom(32).hex())")
```

### シークレットの登録

```bash
curl -X POST http://127.0.0.1:3000/api/secrets \
  -H "Content-Type: application/json" \
  -d '{"name":"DB_PASSWORD","value":"super_secret_123"}'
```

値は AES-256-GCM で暗号化されて DB に保存される。

### シークレットの一覧

```bash
curl http://127.0.0.1:3000/api/secrets
```

値は表示されない。名前と登録日時のみ返却される。

### シークレットの削除

```bash
curl -X DELETE http://127.0.0.1:3000/api/secrets/DB_PASSWORD
```

### ジョブ定義での使用

ノードの `secrets` フィールドで使用するシークレット名を指定する。

```yaml
nodes:
  - id: deploy
    program: bash
    args: ["-c", "./deploy.sh"]
    secrets: ["DB_PASSWORD", "API_KEY"]
```

実行時に指定されたシークレットが復号され、ノードの環境変数として注入される。
上の例では `$DB_PASSWORD` と `$API_KEY` が使用可能になる。

### 注意事項

- 存在しないシークレットを参照するとジョブは失敗する
- 現時点ではシークレット値が実行ログに平文で残る可能性がある（ログマスクは未実装）
- シークレットキーを変更すると既存のシークレットは復号できなくなるため、再登録が必要

## トリガー

### Webhook

外部システム（GitHub, GitLab 等）からの HTTP 呼び出しでジョブを自動実行できる。

#### ジョブ定義

```yaml
version: 1
id: my-ci
name: CI Pipeline
triggers:
  webhook: true
params:
  - name: BRANCH
    default: "main"
defaults:
  working_dir: /opt/repos/sample
  timeout_sec: 300
nodes:
  - id: build
    program: bash
    args: ["-c", "git checkout $BRANCH && make build"]
```

`triggers.webhook: true` を指定すると、Webhook エンドポイントが有効になる。

#### Webhook の呼び出し

```bash
curl -X POST http://127.0.0.1:3000/api/webhook/my-ci \
  -H "Content-Type: application/json" \
  -d '{"params":{"BRANCH":"develop"},"triggered_by":"github"}'
```

- `trigger_type` は自動的に `webhook` に設定される
- `params` でビルドパラメータを渡せる
- `triggered_by` で呼び出し元を記録できる
- リクエストボディは省略可能
- `triggers.webhook` が有効でないジョブへの呼び出しは 400 エラーになる

### Cron スケジュール

ジョブ定義に cron 式を指定すると、スケジュールに従って自動実行される。

#### ジョブ定義

```yaml
version: 1
id: nightly-build
name: Nightly Build
triggers:
  cron: "0 0 3 * * *"    # 毎日 3:00 (秒 分 時 日 月 曜日)
defaults:
  working_dir: /opt/repos/sample
  timeout_sec: 1800
nodes:
  - id: build
    program: make
    args: ["build"]
```

#### cron 式のフォーマット

6 フィールド形式（秒を含む）:

```
秒 分 時 日 月 曜日
```

例:

| 式 | 意味 |
|----|------|
| `0 0 3 * * *` | 毎日 3:00:00 |
| `0 */30 * * * *` | 30 分ごと |
| `0 0 0 * * Mon` | 毎週月曜 0:00 |
| `0 0 9,18 * * Mon-Fri` | 平日 9:00 と 18:00 |

#### 動作仕様

- サーバは 30 秒間隔で全ジョブの cron 式を評価する
- 前回実行時刻以降にスケジュール時刻が到来していれば run を自動作成する
- `trigger_type` は `cron`、`triggered_by` は `scheduler` に設定される
- ジョブが `enabled: false` の場合はトリガーされない
- Webhook と cron は同時に設定できる

## リモート実行

### 概要

postjen-server は単一バイナリでサーバとエージェントの両方の役割を担う。

- 単体起動するとサーバ＋ビルトインローカルエージェントとして動作する
- `--connect-to` を指定すると、上記に加えて別のサーバのリモートエージェントとしても動作する
- リモートエージェントはコントローラーにポーリングしてタスクを取得し、実行結果を報告する（Agent Pull 型）

ジョブ定義の `target` フィールドでエージェント名またはラベルを指定すると、合致するエージェントにノードが割り当てられる。
`target` 未指定のノードはビルトインローカルエージェントで実行される。

### ジョブ定義での実行先指定

`target` フィールドで実行先エージェントを制御する。

#### エージェント名で明示指定

OS やファイルシステムが異なるマシンでは、エージェント名で直接指定する。

```yaml
nodes:
  - id: build-linux
    program: bash
    args: ["-c", "make build"]
    target:
      agent: "linux-builder"

  - id: build-windows
    program: cmd.exe
    args: ["/c", "build.bat"]
    target:
      agent: "windows-builder"
```

#### ラベルでマッチ

同等の役割を持つ複数エージェントから、条件に合うものを自動選択する。

```yaml
nodes:
  - id: test
    program: bash
    args: ["-c", "make test"]
    target:
      labels: ["linux", "gpu"]
```

#### 両方指定（AND 条件）

エージェント名とラベルの両方を指定した場合、名前が一致しかつラベルもすべて合致するエージェントにのみ割り当てられる。

```yaml
target:
  agent: "gpu-server"
  labels: ["cuda"]
```

#### defaults での指定

`defaults.target` を指定すると、全ノードに共通の実行先を設定できる。ノード個別の `target` で上書き可能。

```yaml
defaults:
  working_dir: /opt/repos/sample
  target:
    labels: ["linux"]
nodes:
  - id: test
    program: bash
    args: ["-c", "make test"]
    # → defaults の target が適用される

  - id: gpu-bench
    program: ./bench.sh
    target:
      agent: "gpu-server"         # ノード単位で上書き
    depends_on: ["test"]
```

#### target 未指定

`target` を指定しないノードはコントローラーのビルトインローカルエージェントで実行される。
既存のジョブ定義はそのまま動作する。

### リモート実行の例

#### 1. コントローラーを起動する

```bash
cargo run -p postjen-server
```

#### 2. リモートマシンを接続する

```bash
cargo run -p postjen-server -- \
  --connect-to http://コントローラー:3000 \
  --agent-name linux-builder \
  --agent-labels linux,builder
```

#### 3. コントローラー側でジョブを登録・実行する

```bash
curl -X POST http://コントローラー:3000/api/jobs \
  -H "Content-Type: application/json" \
  -d '{"definition_path":"examples/jobs/sample-remote.yaml","enabled":true}'

curl -X POST http://コントローラー:3000/api/jobs/sample-remote/runs \
  -H "Content-Type: application/json" \
  -d '{"trigger_type":"manual","triggered_by":"user"}'
```

#### 4. 実行状態とログを確認する

```bash
curl http://コントローラー:3000/api/runs/1
curl http://コントローラー:3000/api/runs/1/logs
curl http://コントローラー:3000/api/runs/1/events
```

### エージェント管理 API

#### エージェント一覧

```bash
curl http://127.0.0.1:3000/api/agents
```

期待例:

```json
[
  {
    "agent_id": "local",
    "name": "local",
    "hostname": "controller-host",
    "labels_json": "[\"local\"]",
    "status": "online",
    "last_heartbeat_at": "2026-04-02 08:00:00",
    "registered_at": "2026-04-02 07:50:00"
  },
  {
    "agent_id": "agent-3d19ba8f9b874b3b",
    "name": "linux-builder",
    "hostname": "build-server-01",
    "labels_json": "[\"linux\",\"builder\"]",
    "status": "online",
    "last_heartbeat_at": "2026-04-02 08:00:00",
    "registered_at": "2026-04-02 07:50:00"
  }
]
```

ビルトイン `local` エージェントはサーバ起動時に自動登録される。リモートエージェントは `--connect-to` で接続した際に自動登録される。

#### エージェント詳細

```bash
curl http://127.0.0.1:3000/api/agents/{agent_id}
```

#### エージェント削除

```bash
curl -X DELETE http://127.0.0.1:3000/api/agents/{agent_id}
```

### 成果物（artifacts）

リモートまたはローカルで生成された成果物ファイルは、コントローラー側に自動保存される。

#### 仕組み

1. ジョブ定義でノードの `outputs` に成果物パスを定義する
2. 実行後、`outputs` に定義されたファイルの存在を確認する
3. リモート実行の場合、ファイルをコントローラーにアップロードする
4. ローカル実行の場合、成果物ディレクトリにコピーする
5. コントローラーが `artifacts/{job_run_id}/{node_run_id}/{path}` に保存する
6. `required: true` の成果物が存在しない場合、ノードは `failed` になる

#### ジョブ定義例

```yaml
version: 1
id: remote-build
name: Remote Build with Artifacts
defaults:
  working_dir: /opt/repos/sample
  timeout_sec: 300
nodes:
  - id: build
    name: Build on remote
    program: make
    args: ["build"]
    target:
      agent: "linux-builder"
    outputs:
      - path: dist/app.tar.gz
        required: true
      - path: dist/checksum.txt
        required: false
```

#### 成果物の保存先

保存先はデフォルトで `artifacts/` ディレクトリ（サーバの作業ディレクトリ基準）。
環境変数 `POSTJEN_ARTIFACTS_DIR` で変更できる。

保存時のパス構造:

```
artifacts/
  {job_run_id}/
    {node_run_id}/
      dist/app.tar.gz
      dist/checksum.txt
```

#### 成果物の確認

```bash
ls artifacts/1/1/
cat artifacts/1/1/dist/app.tar.gz
```

#### 注意事項

- `outputs.path` は `working_dir` 基準の相対パスを推奨する
- `required: true`（デフォルト）の成果物が生成されなかった場合、ノードは `failed` になる
- `required: false` の成果物は存在しなくてもノードは成功する

### ハートビートとオフライン検出

リモートエージェントはデフォルトで 15 秒ごとにハートビートを送信する。
コントローラーは 60 秒以上ハートビートが途絶えたエージェントを `offline` と判定し、
そのエージェントに割り当てられていた実行中ノードを `failed` に遷移させる。

ビルトインローカルエージェントはサーバプロセス内で管理されるため、オフラインにはならない。

### 注意事項

- リモートマシン側にもジョブの `working_dir` で指定したパスが存在する必要がある
- OS が異なる場合はエージェント名（`target.agent`）で明示指定し、各 OS 向けのノードを分けて定義する
- リモートマシンの再起動時は新規登録となり、新しいトークンが発行される
- 合致するエージェントがない場合、ノードは即時 `failed` になる
- コントローラーがリモートマシンより先に起動している必要がある

## 現時点の制限

- `POST /api/runs/:run_id/cancel` はキャンセル要求状態への更新までを行う
- `POST /api/runs/:run_id/rerun` は再実行レコード作成までを行う
- `GET /api/runs/:run_id/stream` は現在の状態スナップショットを SSE で返す
- ジョブ間依存は未対応で、連結できるのは同一ジョブ内のノード依存のみ
- エージェント登録は認可なしで誰でも可能（共有シークレットによる制限は未実装）
- エージェントの負荷分散は未実装（合致する最初のエージェントに割り当て）
- シークレット値が実行ログに平文で残る可能性がある（ログマスクは未実装）
- 条件付き実行（`when` による実行スキップ）は未対応

## 補足

- DB は `SQLite` を使用する
- ジョブ定義の履歴管理は DB ではなく `git` 等に委ねる
- 実行制約や状態遷移仕様は [implementation-policy.md](docs/implementation-policy.md) を参照する
