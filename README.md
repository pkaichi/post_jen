# postjen

Jenkins 代替として作成された、Rust 製のジョブ実行サービス。

YAML で定義したジョブを DAG 依存関係に基づいて実行し、REST API および Web UI でジョブの管理・実行・監視を行う。
プラグイン等の拡張機構は持たず、ジョブの実行と管理に特化したシンプルな設計。

## 主な機能

- **YAML ベースのジョブ定義** — ノード間の DAG 依存関係・並列実行をサポート
- **REST API** — ジョブ登録・実行・監視・キャンセル・再実行
- **Web UI** — ダッシュボード・ジョブ詳細・ログビューア（Leptos/WASM、サーババイナリに組込み）
- **リモートエージェント** — 複数マシンへの分散実行（`--connect-to` で接続）
- **cron / webhook トリガー** — スケジュール実行・外部トリガー
- **ビルドパラメータ** — 実行時にパラメータを注入
- **シークレット管理** — AES-256-GCM 暗号化で安全に保管・ノードへ注入
- **アーティファクト管理** — ノード出力ファイルの収集・保存
- **SSE ストリーミング** — リアルタイムログ・イベント監視

## セットアップ

### 必要なもの

- Rust ツールチェイン（`rustup` 経由で導入）

### インストール・起動

```bash
git clone git@github.com:pkaichi/post_jen.git
cd post_jen
cargo run -p postjen-server
```

起動時に以下が自動的に行われる:

- SQLite スキーマの初期化（`db/schema.sql`）
- ビルトインローカルエージェントの登録
- 成果物ディレクトリの作成

### 動作確認

```bash
curl http://127.0.0.1:3000/api/health
# => {"status":"ok"}
```

Web UI はブラウザで `http://127.0.0.1:3000/` にアクセスする。

### リモートエージェントとして起動

別マシンのコントローラに接続する場合:

```bash
cargo run -p postjen-server -- \
  --connect-to http://<controller>:3000 \
  --agent-name linux-builder \
  --agent-labels linux,gpu
```

## 環境変数

| 変数 | デフォルト | 説明 |
|------|-----------|------|
| `POSTJEN_BIND_ADDR` | `127.0.0.1:3000` | リッスンアドレス |
| `POSTJEN_DATABASE_URL` | `sqlite:postjen.db` | SQLite パス |
| `POSTJEN_ARTIFACTS_DIR` | `artifacts` | 成果物の保存先 |
| `POSTJEN_SECRET_KEY` | なし | シークレット暗号化キー（32 バイト hex） |

## 構成

```
crates/
  postjen-core/      # ジョブ定義パーサ・バリデーション
  postjen-server/    # API サーバ・実行エンジン・エージェント
  postjen-ui/        # Web UI（Leptos/WASM）
db/
  schema.sql         # SQLite DDL
examples/
  jobs/              # サンプルジョブ定義 YAML
```

## ドキュメント

- [usage.md](usage.md) — API 仕様・curl 例
- [docs/implementation-policy.md](docs/implementation-policy.md) — 設計方針
- [docs/work-log.md](docs/work-log.md) — 開発経緯
