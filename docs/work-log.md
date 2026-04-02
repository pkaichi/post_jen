# Work Log

## 概要

このファイルは、`postjen` の実装作業でここまでに行った内容を時系列で残すためのログである。
設計の詳細は [implementation-policy.md](/mnt/c/Users/pkaichi/workspace/postjen/postjen_proj/docs/implementation-policy.md) を参照する。

## 実施ログ

### 1. 初期セットアップ

- Rust workspace と `postjen-server` の雛形を作成
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

- [sample-dag-success.yaml](/mnt/c/Users/pkaichi/workspace/postjen/postjen_proj/examples/jobs/sample-dag-success.yaml)
- [sample-failure.yaml](/mnt/c/Users/pkaichi/workspace/postjen/postjen_proj/examples/jobs/sample-failure.yaml)
- [sample-timeout.yaml](/mnt/c/Users/pkaichi/workspace/postjen/postjen_proj/examples/jobs/sample-timeout.yaml)

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

### 8. 次期機能の概要設計

リモートエージェント連携と Web UI の概要設計ドキュメントを作成した。

追加したドキュメント:

- [remote-agent-design.md](/docs/remote-agent-design.md)
  - Agent Pull 型アーキテクチャを採用
  - `postjen-agent` を新規バイナリとしてワークスペースに追加する構成
  - ジョブ定義に `target.labels` を追加してエージェント割当を制御
  - `target` 未指定ノードは従来通りローカル実行（後方互換）
  - 通信フロー: 登録 → ポーリング → ログ送信 → 結果報告 → ハートビート
  - DB 拡張: `agents` テーブル追加、`node_runs` に `assigned_agent_id` 追加
  - 4 フェーズの段階的導入計画

- [web-ui-design.md](/docs/web-ui-design.md)
  - 6 画面構成: ダッシュボード、ジョブ一覧、ジョブ詳細、実行詳細、ログ表示、エージェント一覧
  - 各画面で使用する既存 API を整理し、追加が必要な API を明確化
  - 既存 SSE を活用したリアルタイム更新方針
  - フレームワークは未確定、選定時の考慮点のみ記載
  - 3 フェーズの段階的導入計画

### 9. プロジェクト名リネーム（postgen → postjen）

Jenkins 後継の意図に合わせて、プロジェクト全体の名称を `postgen` から `postjen` に統一した。

変更内容:

- ディレクトリ名: `crates/postgen-server` → `crates/postjen-server`
- パッケージ名: `postgen-server` → `postjen-server`
- 環境変数: `POSTGEN_BIND_ADDR` → `POSTJEN_BIND_ADDR`, `POSTGEN_DATABASE_URL` → `POSTJEN_DATABASE_URL`
- DB ファイル名: `postgen.db` → `postjen.db`
- 全ドキュメント・サンプル YAML 内の表記を修正
- `db.rs` に `create_if_missing(true)` を追加（DB 未作成時の起動エラー修正）

関連ブランチとコミット:

- `feature/proj_rename`

### 10. リモートエージェント機能の実装

[remote-agent-design.md](/docs/remote-agent-design.md) に基づき、Agent Pull 型のリモート実行機能を実装した。

#### クレート構成の変更

- `crates/postjen-core` を新設（共有ライブラリ）
  - `definition.rs` — ジョブ定義パース・バリデーション・トポロジカルソート（サーバから移動）
  - `executor.rs` — ノード実行ロジック（`run_process`, `check_outputs`）を DB 非依存で切り出し
  - `types.rs` — `NodeExecutionOutcome`, `ArtifactResult` 等の共通型
- `crates/postjen-agent` を新設（エージェントバイナリ）
  - `client.rs` — サーバとの HTTP 通信（reqwest）
  - `worker.rs` — ポーリング・実行・結果報告・ハートビートのループ処理
  - CLI 引数: `--server-url`, `--name`, `--labels`, `--poll-interval`, `--heartbeat-interval`

#### DB スキーマ拡張

- `agents` テーブル追加（agent_id, name, hostname, labels_json, status, token_hash, last_heartbeat_at）
- `node_runs` に `target_json`, `assigned_agent_id` カラム追加

#### API 拡張

エージェント管理 API（サーバ管理者向け）:

- `GET /api/agents` — エージェント一覧
- `POST /api/agents` — エージェント登録（トークン発行）
- `GET /api/agents/:agent_id` — エージェント詳細
- `DELETE /api/agents/:agent_id` — エージェント削除

エージェント用 API（エージェントプロセスが使用、Bearer トークン認証）:

- `GET /api/agent/task` — 割り当てられたタスクをポーリング（200 or 204）
- `POST /api/agent/result` — ノード実行結果を報告
- `POST /api/agent/logs` — 実行ログをバッチ送信
- `POST /api/agent/heartbeat` — ハートビート送信

#### ジョブ定義の拡張

- `defaults.target` および `nodes[].target` に `labels` フィールドを追加
- `target` 未指定ノードは従来通りサーバでローカル実行（後方互換）

#### Runner スケジューリング拡張

- `target` 付きノードはラベルが合致する online エージェントに割り当て
- `wait_for_remote_node` でエージェントの結果報告をポーリング待機
- リモートノード完了後、依存する後続ノードの実行を続行
- ハートビート監視（15 秒間隔）でオフラインエージェントを検出し、該当ノードを `failed` に遷移

#### E2E テスト結果

`examples/jobs/sample-remote.yaml` を用いて、サーバ + エージェントの連携を localhost 上で検証した。

- エージェント登録: 起動時に自動登録、`GET /api/agents` で status=online を確認
- リモート実行: `remote-hello` ノードがエージェントで実行され、ログがサーバに記録された
- 依存実行: `local-after-remote` ノードがリモートノード完了後にサーバでローカル実行された
- ジョブ全体: status=success で完了
- 状態遷移イベント: 全 9 イベントが正しい順序で記録された

### 11. 成果物アップロード・エージェント名指定・target.agent 対応

- リモートで生成された成果物をコントローラーにアップロードする機能を追加
  - `POST /api/agent/artifacts`（マルチパート）でファイルを受信
  - コントローラー側に `artifacts/{job_run_id}/{node_run_id}/{path}` として保存
  - タスク情報に `outputs` 定義を含めてエージェントに送信
  - エージェントが実行後に成果物の存在確認＋アップロードを実行
  - `required: true` の成果物が欠落した場合はノードを `failed` にする
- `target.agent` フィールドを追加し、エージェント名で明示的に実行先を指定可能にした
  - OS やファイルシステムが異なるマシンへの振り分けを想定
  - `target.labels` との AND 条件にも対応
- `POSTJEN_ARTIFACTS_DIR` 環境変数を追加（デフォルト: `artifacts`）
- `.gitignore` に `artifacts/` を追加

### 12. 実行パスの統一（ビルトインローカルエージェント）

runner.rs のローカル実行パスを廃止し、全ノードをエージェント経由で実行するよう統一した。

変更内容:

- サーバ起動時にビルトイン `local` エージェントを DB に自動登録
- `target` 未指定ノードはビルトイン `local` エージェントに自動割り当て
- ローカルワーカーがインプロセスで `local` エージェント宛タスクを DB から取得・実行
- runner.rs の `execute_node()` 等の直接実行コードを削除
- 実行パスが「スケジューラ → エージェント割当 → ワーカー実行」に一本化

### 13. サーバ・エージェント統合（単一バイナリ化）

`postjen-agent` クレートを廃止し、エージェント機能を `postjen-server` に統合した。

変更内容:

- `postjen-agent` の `client.rs`、`worker.rs` を `postjen-server` に `agent_client.rs`、`agent_worker.rs` として移動
- `main.rs` に `clap` による CLI 引数パースを追加
- `--connect-to` オプションで他サーバのリモートエージェントとして接続可能に
- `--agent-name`、`--agent-labels`、`--poll-interval`、`--heartbeat-interval` オプション追加
- `crates/postjen-agent` ディレクトリを削除、ワークスペースから除外

動作モデル:

- 単体起動: サーバ＋ビルトインローカルエージェント
- `--connect-to` 付き起動: 上記に加えて指定コントローラーのリモートエージェントとしても動作
- 全インスタンスがサーバでありエージェントでもある（Jenkins と同様の分散モデル）
- 接続はリモート → コントローラーへの Pull 型。コントローラーのポートのみ開放すればよい

E2E テスト結果:

- コントローラー（:3000）とリモート（:3001）を同一マシンで起動
- `target.agent: "linux-builder"` のノードがリモートで実行され `executed on remote server` を確認
- `target` なしのノードがコントローラーで実行され `executed on controller` を確認
- ジョブ全体: status=success

### 14. usage.md 全面改訂

統一アーキテクチャに合わせて usage.md を全面書き換えた。

- 環境変数を `POSTJEN_*` に修正、`POSTJEN_ARTIFACTS_DIR` を追加
- 起動セクション: 単体起動と `--connect-to` 付き起動を統一的に説明
- 複数マシンでの分散実行の構成例を追加
- リモート実行セクション: `postjen-agent` の記述を全削除、単一バイナリアーキテクチャに書き換え
- 成果物セクション: ローカル/リモート両方の保存フローを統一的に記載
- ファイルパス: WSL 固有の絶対パスを相対パスに修正

## 現時点の状態

できること:

- ジョブ定義 YAML の登録
- 単一ジョブの実行（ビルトインローカルエージェント経由）
- 同一ジョブ内ノードの依存付き順次実行
- 実行履歴、イベント、ログの取得
- 再実行レコードの作成
- キャンセル要求状態への更新
- リモートエージェントへのノード実行委譲（Agent Pull 型）
- `target.agent` によるエージェント名指定、`target.labels` によるラベルマッチ
- エージェントの登録・管理・ハートビート監視・オフライン検出
- 成果物のエージェント→コントローラーアップロード
- 単一バイナリ（`postjen-server`）でサーバ＋エージェントの両役割

未対応または今後の検討項目:

- ジョブ間依存
- ノード並列実行
- 高度な再試行制御
- Web UI の整備
- 定義同期の自動化
- 認証の強化（現在は共有シークレットなし、トークンのみ）
- エージェントの自動再登録（再起動時は新規登録になる）
- 負荷分散の改善（現在は最初にマッチしたエージェントに割当）

## メモ

- サンプル実行用の生成物は `examples/sample-work/` などローカル出力で扱う
- 生成物の ignore 設定と機能追加は、今後はコミットを分ける方針
- 既存サンプル（sample-hello 等）の `working_dir` は WSL 向けパスのため、macOS では実行不可
- コントローラーがリモートマシンより先に起動している必要がある

## Next Actions

優先度順の次アクション:

1. Web UI Phase 1: フレームワーク選定、ダッシュボード（実行一覧）と実行詳細画面の実装
2. 認証の強化: エージェント登録時の共有シークレット認可を追加
3. エージェント負荷分散の改善（ラウンドロビン等）
4. エージェントの自動再登録（切断後の復帰対応）
5. 既存サンプルの `working_dir` を環境非依存なパスに修正
