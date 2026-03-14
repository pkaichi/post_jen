# Work Log

## 概要

このファイルは、`postgen` の実装作業でここまでに行った内容を時系列で残すためのログである。
設計の詳細は [implementation-policy.md](/mnt/c/Users/pkaichi/workspace/postjen/postgen_proj/docs/implementation-policy.md) を参照する。

## 実施ログ

### 1. 初期セットアップ

- Rust workspace と `postgen-server` の雛形を作成
- `axum` ベースの HTTP サーバを追加
- SQLite 初期化と `db/schema.sql` の読込を追加
- `README.md` と `usage.md` の初期版を整備

関連コミット:

- `dd66b01` `Initial project setup`

### 2. API 実装

- `GET /api/jobs`
- `GET /api/jobs/:job_id`
- `POST /api/jobs/:job_id/runs`
- `GET /api/runs`
- `GET /api/runs/:run_id`
- `POST /api/runs/:run_id/cancel`
- `POST /api/runs/:run_id/rerun`
- `GET /api/runs/:run_id/logs`
- `GET /api/runs/:run_id/events`
- `GET /api/runs/:run_id/stream`

補足:

- `start_run` は実行レコードを `queued` で作成する
- `cancel_run` は `cancel_requested` への状態更新までを行う
- `rerun_run` は新規 run を再作成する
- `stream_run` は SSE で状態スナップショットを返す

関連ブランチとコミット:

- `feature/api-implementation`
- `76fd3f8` `Implement run APIs`

### 3. 実行エンジン実装

- バックグラウンドワーカーを追加
- `queued` の `job_runs` を拾って実行する処理を追加
- YAML ジョブ定義の読込と検証を追加
- DAG 順序に沿ったノード実行を追加
- `run_events`、`run_logs`、`run_artifacts` の記録を追加
- タイムアウト、失敗、キャンセルの基本遷移を追加

補足:

- 実行方式は現時点では順次実行
- 並列実行、複雑な再試行制御、ジョブ間依存は未対応

関連ブランチとコミット:

- `feature/execution-engine`
- `78d8ddb` `Implement execution engine`

### 4. ジョブ定義登録 API

- `POST /api/jobs` を追加
- `definition_path` から YAML を読んで `job_definitions` へ upsert する処理を追加
- `definition_hash` を SHA-256 で保存するようにした
- `usage.md` と `README.md` を更新

関連ブランチとコミット:

- `feature/job-definition-registration`
- `a47d914` `Add job definition registration API and persist YAML metadata`

### 5. サンプル作成と疎通確認

- `examples/jobs/sample-hello.yaml` を追加
- サンプル登録、実行、ログ確認、成果物生成までを確認
- 実行生成物は `examples/sample-work/` に出力する構成とした

確認した内容:

- `POST /api/jobs` で登録できること
- `POST /api/jobs/:job_id/runs` で実行できること
- `GET /api/runs/:run_id` で状態確認できること
- `GET /api/runs/:run_id/logs` と `events` が記録されること

関連ブランチとコミット:

- `feature/sample-hello-job`
- `6becf23` `Add sample hello job definition for end-to-end execution check`

### 6. `job_runs.working_dir` 修正

- `job_runs.working_dir` に定義ファイル親ディレクトリが入っていた問題を修正
- 実際のジョブ定義から解決した `working_dir` を実行スナップショットに保存するよう変更
- `sample-hello` 再実行で `working_dir` が期待通りになることを確認

関連ブランチとコミット:

- `feature/fix-job-run-working-dir`
- `1570de1` `Store resolved job working_dir in job run snapshots`

### 7. 追加サンプルと使い方整理

追加したサンプル:

- [sample-dag-success.yaml](/mnt/c/Users/pkaichi/workspace/postjen/postgen_proj/examples/jobs/sample-dag-success.yaml)
- [sample-failure.yaml](/mnt/c/Users/pkaichi/workspace/postjen/postgen_proj/examples/jobs/sample-failure.yaml)
- [sample-timeout.yaml](/mnt/c/Users/pkaichi/workspace/postjen/postgen_proj/examples/jobs/sample-timeout.yaml)

更新内容:

- `usage.md` に現状の基本フローを整理
- サンプル一覧と用途説明を追加
- ジョブ間依存が未対応であることを明記

関連ブランチとコミット:

- `feature/add-sample-job-docs`
- `db7c236` `Add sample job definitions and document current usage flow`

## 現時点の状態

できること:

- ジョブ定義 YAML の登録
- 単一ジョブの実行
- 同一ジョブ内ノードの依存付き順次実行
- 実行履歴、イベント、ログの取得
- 再実行レコードの作成
- キャンセル要求状態への更新

未対応または今後の検討項目:

- ジョブ間依存
- ノード並列実行
- 高度な再試行制御
- UI の整備
- 定義同期の自動化
- 認証、権限制御、運用面の設計整理

## メモ

- サンプル実行用の生成物は `examples/sample-work/` などローカル出力で扱う
- 生成物の ignore 設定と機能追加は、今後はコミットを分ける方針

## Next Actions

優先度順の次アクション:

1. 追加したサンプルを実際に実行して、`success` / `failed` / `timed_out` の各ケースを確認する
2. サンプル実行結果を `usage.md` か別ドキュメントへ追記し、期待される挙動を明文化する
3. MVP 後の設計整理として、ジョブ間依存を入れるか、1 ジョブ内 DAG に寄せるかを決める
4. Jenkins 代替を見据えて、並列実行、キュー制御、定義同期の設計を整理する
