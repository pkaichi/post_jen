# Usage

## 前提

このプロジェクトを実行するには、以下が必要です。

- `cargo`
- `rustc`

通常は `rustup` 経由で Rust ツールチェインを導入する。

## 環境変数

- `POSTGEN_BIND_ADDR`
  - 省略時は `127.0.0.1:3000`
- `POSTGEN_DATABASE_URL`
  - 省略時は `sqlite:postjen.db`

例:

```bash
export POSTGEN_BIND_ADDR=127.0.0.1:3000
export POSTGEN_DATABASE_URL=sqlite:postjen.db
```

## 実行手順

1. Rust ツールチェインを導入する
2. プロジェクトルートへ移動する
3. サーバを起動する

```bash
cd /mnt/c/Users/pkaichi/workspace/postjen/postjen_proj
cargo run -p postjen-server
```

起動すると、SQLite スキーマは [schema.sql](/mnt/c/Users/pkaichi/workspace/postjen/postjen_proj/db/schema.sql) を元に自動初期化される。

## 基本フロー

現状の基本的な使い方は次の流れです。

1. ジョブ定義 YAML を用意する
2. `POST /api/jobs` でジョブ定義を登録する
3. `POST /api/jobs/:job_id/runs` で実行を作成する
4. `GET /api/runs/:run_id` `logs` `events` で状態を確認する

## サンプル定義

リポジトリには以下のサンプルを用意している。

- [sample-hello.yaml](/mnt/c/Users/pkaichi/workspace/postjen/postjen_proj/examples/jobs/sample-hello.yaml)
  - 1 ノード成功の最小サンプル
  - ファイル出力と成果物確認を行う
- [sample-dag-success.yaml](/mnt/c/Users/pkaichi/workspace/postjen/postjen_proj/examples/jobs/sample-dag-success.yaml)
  - 依存関係付き 2 ノード成功サンプル
  - `depends_on` による順次実行を確認できる
- [sample-failure.yaml](/mnt/c/Users/pkaichi/workspace/postjen/postjen_proj/examples/jobs/sample-failure.yaml)
  - 失敗ノードと後続 `skipped` の確認用サンプル
- [sample-timeout.yaml](/mnt/c/Users/pkaichi/workspace/postjen/postjen_proj/examples/jobs/sample-timeout.yaml)
  - ノードタイムアウト遷移の確認用サンプル

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
  -d '{"definition_path":"/mnt/c/Users/pkaichi/workspace/postjen/postjen_proj/examples/jobs/sample-hello.yaml","enabled":true}'
```

期待例:

```json
{
  "job_id":"sample-hello",
  "name":"Sample Hello Job",
  "description":"Create a sample artifact and emit a simple log",
  "definition_path":"/mnt/c/Users/pkaichi/workspace/postjen/postjen_proj/examples/jobs/sample-hello.yaml",
  "definition_hash":"...",
  "enabled":1,
  "created_at":"2026-03-14 12:00:00",
  "updated_at":"2026-03-14 12:00:00"
}
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

## リモートエージェント

### 概要

postjen-agent を別マシン（または同一マシンの別プロセス）で起動し、ジョブのノードをリモートで実行できる。
エージェントはサーバにポーリングしてタスクを取得し、実行結果をサーバに報告する（Agent Pull 型）。

ジョブ定義の `target` フィールドでエージェント名またはラベルを指定すると、合致するエージェントにノードが割り当てられる。
`target` 未指定のノードは従来通りサーバ上でローカル実行される。

### エージェントの起動

```bash
cargo run -p postjen-agent -- \
  --server-url http://サーバアドレス:3000 \
  --name linux-builder \
  --labels linux,builder
```

| オプション | 必須 | 説明 |
|-----------|------|------|
| `--server-url` | 必須 | postjen-server の URL |
| `--name` | 必須 | エージェントの表示名。`target.agent` で指定する際に使用する |
| `--labels` | 任意 | カンマ区切りのラベル。`target.labels` でマッチに使用する |
| `--poll-interval` | 任意 | タスクポーリング間隔（秒）。デフォルト: 2 |
| `--heartbeat-interval` | 任意 | ハートビート送信間隔（秒）。デフォルト: 15 |

エージェントは起動時にサーバへ自動登録され、トークンを取得する。
以後はこのトークンで認証してサーバと通信する。

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

`target` を指定しないノードは従来通りサーバ上でローカル実行される。
既存のジョブ定義はそのまま動作する。

### エージェント管理 API

#### エージェント一覧

```bash
curl http://127.0.0.1:3000/api/agents
```

期待例:

```json
[
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

#### エージェント登録（手動）

通常はエージェント起動時に自動登録されるが、手動で登録することもできる。

```bash
curl -X POST http://127.0.0.1:3000/api/agents \
  -H "Content-Type: application/json" \
  -d '{"name":"manual-agent","hostname":"localhost","labels":["test"]}'
```

期待例:

```json
{"agent_id":"agent-xxxx","token":"yyyy..."}
```

#### エージェント詳細

```bash
curl http://127.0.0.1:3000/api/agents/agent-3d19ba8f9b874b3b
```

#### エージェント削除

```bash
curl -X DELETE http://127.0.0.1:3000/api/agents/agent-3d19ba8f9b874b3b
```

### リモート実行の例

以下の手順で、サーバとエージェントを連携させてジョブを実行する。

#### 1. サーバを起動する

```bash
cargo run -p postjen-server
```

#### 2. エージェントを起動する

```bash
cargo run -p postjen-agent -- \
  --server-url http://127.0.0.1:3000 \
  --name local-agent \
  --labels test,builder
```

#### 3. リモート実行を含むジョブを登録・実行する

```bash
curl -X POST http://127.0.0.1:3000/api/jobs \
  -H "Content-Type: application/json" \
  -d '{"definition_path":"examples/jobs/sample-remote.yaml","enabled":true}'

curl -X POST http://127.0.0.1:3000/api/jobs/sample-remote/runs \
  -H "Content-Type: application/json" \
  -d '{"trigger_type":"manual","triggered_by":"user"}'
```

#### 4. 実行状態とログを確認する

```bash
curl http://127.0.0.1:3000/api/runs/1
curl http://127.0.0.1:3000/api/runs/1/logs
curl http://127.0.0.1:3000/api/runs/1/events
```

### サンプル定義

- [sample-remote.yaml](examples/jobs/sample-remote.yaml)
  - `target.labels` でリモート実行するノードと、ローカル実行する後続ノードの組み合わせ
  - 依存関係によりリモートノード完了後にローカルノードが実行される

### 成果物（artifacts）のサーバ受け取り

エージェントで生成された成果物ファイルは、サーバ側に自動アップロードされる。

#### 仕組み

1. ジョブ定義でノードの `outputs` に成果物パスを定義する
2. サーバがタスクをエージェントに渡す際、`outputs` 定義も含めて送信する
3. エージェントがプロセス実行後、`outputs` に定義されたファイルの存在を確認する
4. 存在するファイルをサーバの `POST /api/agent/artifacts` にアップロードする
5. サーバが `artifacts/{job_run_id}/{node_run_id}/{path}` に保存する
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

サーバ側の保存先はデフォルトで `artifacts/` ディレクトリ（サーバの作業ディレクトリ基準）。
環境変数 `POSTJEN_ARTIFACTS_DIR` で変更できる。

```bash
export POSTJEN_ARTIFACTS_DIR=/var/postjen/artifacts
```

保存時のパス構造:

```
artifacts/
  {job_run_id}/
    {node_run_id}/
      dist/app.tar.gz        # outputs.path がそのまま使われる
      dist/checksum.txt
```

#### 成果物の確認

実行完了後、サーバのファイルシステム上で直接確認できる。

```bash
# 保存されたファイルを確認
ls artifacts/1/1/

# 中身を確認
cat artifacts/1/1/dist/app.tar.gz
```

#### 注意事項

- `outputs.path` は `working_dir` 基準の相対パスを推奨する
- 絶対パスの場合はそのまま使用される
- `required: true`（デフォルト）の成果物が生成されなかった場合、ノードは `failed` になる
- `required: false` の成果物は存在しなくてもノードは成功する
- ローカル実行の場合も `outputs` による存在確認は行われるが、ファイル転送は発生しない（サーバ上で直接参照できるため）

### ハートビートとオフライン検出

エージェントはデフォルトで 15 秒ごとにハートビートを送信する。
サーバは 60 秒以上ハートビートが途絶えたエージェントを `offline` と判定し、
そのエージェントに割り当てられていた実行中ノードを `failed` に遷移させる。

### 注意事項

- エージェント側にもジョブの `working_dir` で指定したパスが存在する必要がある
- OS が異なる場合はエージェント名で明示指定し、各 OS 向けのノードを分けて定義する
- エージェント再起動時は新規登録となり、新しいトークンが発行される
- 合致するエージェントがない場合、ノードは即時 `failed` になる

## 現時点の制限

- `POST /api/runs/:run_id/cancel` はキャンセル要求状態への更新までを行う
- `POST /api/runs/:run_id/rerun` は再実行レコード作成までを行う
- `GET /api/runs/:run_id/stream` は現在の状態スナップショットを SSE で返す
- 実行エンジンは順次実行の MVP 実装であり、並列実行や高度な再試行制御は未対応
- ジョブ間依存は未対応で、連結できるのは同一ジョブ内のノード依存のみ
- エージェント登録は認可なしで誰でも可能（共有シークレットによる制限は未実装）
- エージェントの負荷分散は未実装（合致する最初のエージェントに割り当て）
- リモートノードの成果物チェック（`outputs`）はエージェント側で未対応

## 補足

- DB は `SQLite` を使用する
- ジョブ定義の履歴管理は DB ではなく `git` 等に委ねる
- 実行制約や状態遷移仕様は [implementation-policy.md](/mnt/c/Users/pkaichi/workspace/postjen/postjen_proj/docs/implementation-policy.md) を参照する
