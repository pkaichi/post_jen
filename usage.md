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
  - 省略時は `sqlite:postgen.db`

例:

```bash
export POSTGEN_BIND_ADDR=127.0.0.1:3000
export POSTGEN_DATABASE_URL=sqlite:postgen.db
```

## 実行手順

1. Rust ツールチェインを導入する
2. プロジェクトルートへ移動する
3. サーバを起動する

```bash
cd /mnt/c/Users/pkaichi/workspace/postjen/postgen_proj
cargo run -p postgen-server
```

起動すると、SQLite スキーマは [schema.sql](/mnt/c/Users/pkaichi/workspace/postjen/postgen_proj/db/schema.sql) を元に自動初期化される。

## 基本フロー

現状の基本的な使い方は次の流れです。

1. ジョブ定義 YAML を用意する
2. `POST /api/jobs` でジョブ定義を登録する
3. `POST /api/jobs/:job_id/runs` で実行を作成する
4. `GET /api/runs/:run_id` `logs` `events` で状態を確認する

## サンプル定義

リポジトリには以下のサンプルを用意している。

- [sample-hello.yaml](/mnt/c/Users/pkaichi/workspace/postjen/postgen_proj/examples/jobs/sample-hello.yaml)
  - 1 ノード成功の最小サンプル
  - ファイル出力と成果物確認を行う
- [sample-dag-success.yaml](/mnt/c/Users/pkaichi/workspace/postjen/postgen_proj/examples/jobs/sample-dag-success.yaml)
  - 依存関係付き 2 ノード成功サンプル
  - `depends_on` による順次実行を確認できる
- [sample-failure.yaml](/mnt/c/Users/pkaichi/workspace/postjen/postgen_proj/examples/jobs/sample-failure.yaml)
  - 失敗ノードと後続 `skipped` の確認用サンプル
- [sample-timeout.yaml](/mnt/c/Users/pkaichi/workspace/postjen/postgen_proj/examples/jobs/sample-timeout.yaml)
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
  -d '{"definition_path":"/mnt/c/Users/pkaichi/workspace/postjen/postgen_proj/examples/jobs/sample-hello.yaml","enabled":true}'
```

期待例:

```json
{
  "job_id":"sample-hello",
  "name":"Sample Hello Job",
  "description":"Create a sample artifact and emit a simple log",
  "definition_path":"/mnt/c/Users/pkaichi/workspace/postjen/postgen_proj/examples/jobs/sample-hello.yaml",
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

## 現時点の制限

- `POST /api/runs/:run_id/cancel` はキャンセル要求状態への更新までを行う
- `POST /api/runs/:run_id/rerun` は再実行レコード作成までを行う
- `GET /api/runs/:run_id/stream` は現在の状態スナップショットを SSE で返す
- 実行エンジンは順次実行の MVP 実装であり、並列実行や高度な再試行制御は未対応
- ジョブ間依存は未対応で、連結できるのは同一ジョブ内のノード依存のみ

## 補足

- DB は `SQLite` を使用する
- ジョブ定義の履歴管理は DB ではなく `git` 等に委ねる
- 実行制約や状態遷移仕様は [implementation-policy.md](/mnt/c/Users/pkaichi/workspace/postjen/postgen_proj/docs/implementation-policy.md) を参照する
