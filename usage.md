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

### ジョブ詳細取得

```bash
curl http://127.0.0.1:3000/api/jobs/sample-build
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

現状の雛形では、以下の API は stub のみで `501 Not Implemented` を返す。

- `POST /api/jobs/:job_id/runs`
- `POST /api/runs/:run_id/cancel`
- `POST /api/runs/:run_id/rerun`
- `GET /api/runs/:run_id/stream`

## 補足

- DB は `SQLite` を使用する
- ジョブ定義の履歴管理は DB ではなく `git` 等に委ねる
- 実行制約や状態遷移仕様は [implementation-policy.md](/mnt/c/Users/pkaichi/workspace/postjen/postgen_proj/docs/implementation-policy.md) を参照する
