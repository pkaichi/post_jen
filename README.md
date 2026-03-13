# postgen

Rust で実装するジョブ実行サービスの MVP 雛形です。

## セットアップ

リポジトリを clone して起動確認するまでの最短手順です。

```bash
git clone git@github.com:pkaichi/post_jen.git
cd post_jen
sudo apt update
sudo apt install -y build-essential pkg-config libssl-dev
curl https://sh.rustup.rs -sSf | sh -s -- -y
. "$HOME/.cargo/env"
cargo run -p postgen-server
```

起動後の確認:

```bash
curl http://127.0.0.1:3000/api/health
```

期待例:

```json
{"status":"ok"}
```

補足:

- DB は既定で `sqlite:postgen.db` を使用する
- 起動時に `db/schema.sql` を元に SQLite スキーマを自動初期化する
- 環境変数などの詳細は `usage.md` を参照する

## 構成

- `crates/postgen-server`
  - `axum` ベースの API サーバ
- `db/schema.sql`
  - `SQLite` 初期 DDL
- `docs/implementation-policy.md`
  - 実装方針と設計メモ

## 現状

- `SQLite` スキーマを含む
- API の主要ルートを実装済み
- 実行処理そのものはこれから拡張する段階
