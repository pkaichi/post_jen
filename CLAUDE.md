# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## プロジェクト概要

**postjen** — Rust製のジョブ実行サービス（Jenkins代替MVP）。YAML定義のジョブをDAG依存関係に基づいて逐次実行し、REST APIでジョブ管理・実行・監視を行う。

## ビルド・実行コマンド

```bash
# ビルド
cargo build -p postjen-server

# 実行（デフォルト: 127.0.0.1:3000, sqlite:postjen.db）
cargo run -p postjen-server

# ヘルスチェック
curl http://127.0.0.1:3000/api/health

# ログレベル変更
RUST_LOG=debug cargo run -p postjen-server
```

自動テストは未整備。手動テストは `usage.md` の curl 例と `examples/jobs/` のサンプルYAMLで実施する。

## 環境変数

| 変数 | デフォルト | 説明 |
|------|-----------|------|
| `POSTGEN_BIND_ADDR` | `127.0.0.1:3000` | リッスンアドレス |
| `POSTGEN_DATABASE_URL` | `sqlite:postjen.db` | SQLiteパス |
| `RUST_LOG` | `info` | ログレベル |

## アーキテクチャ

Cargoワークスペース構成。メンバーは `crates/postjen-server` のみ。

### ソースモジュール (`crates/postjen-server/src/`)

- **main.rs** — エントリポイント。tracing初期化 → Config読込 → DB接続 → バックグラウンドワーカー起動 → Axumサーバ起動
- **config.rs** — 環境変数からの設定読込
- **db.rs** — SQLiteプール生成とスキーマ初期化（`db/schema.sql` を埋め込み実行）
- **definition.rs** — ジョブ定義YAMLの読込・バリデーション・トポロジカルソート
- **runner.rs** — バックグラウンド実行エンジン。1秒間隔でキューをポーリングし、`tokio::process::Command` でノードを逐次実行
- **http.rs** — Axumルーター・全APIエンドポイント・リクエスト/レスポンス型定義

### データフロー

1. `POST /api/jobs` でYAMLファイルパスを指定しジョブ定義をDBに登録
2. `POST /api/jobs/:job_id/runs` で実行を開始（`created` → `queued`）
3. runner がキューから取得し、トポロジカル順にノードを実行
4. 各ノードの stdout/stderr は `run_logs` に、状態遷移は `run_events` に記録
5. `GET /api/runs/:run_id/stream` でSSEによるリアルタイム監視

### DBスキーマ (`db/schema.sql`)

SQLite。7テーブル: `job_definitions`, `job_runs`, `node_runs`, `run_events`, `run_logs`, `run_artifacts` + インデックス群。起動時に自動作成される。

### ジョブ実行の状態遷移

- **Job Run**: `created → queued → running → {success | failed | timed_out | canceled}`
- **Node Run**: `pending → queued → running → {success | failed | timed_out | canceled | skipped}`
- 依存先が失敗した場合、後続ノードは `skipped` になる

## ジョブ定義YAMLフォーマット

`examples/jobs/` にサンプルあり。version 1形式。主要フィールド:
- `id` — 小文字英数字+ハイフン/アンダースコア
- `nodes[].program` — 実行プログラム（パスまたはコマンド名）
- `nodes[].depends_on` — DAG依存関係
- `defaults` — 全ノード共通のworking_dir, timeout_sec, env等

## ドキュメント

- `usage.md` — API仕様・curl例（日本語）
- `docs/implementation-policy.md` — 設計方針・アーキテクチャ決定
- `docs/work-log.md` — 開発経緯・作業ログ
