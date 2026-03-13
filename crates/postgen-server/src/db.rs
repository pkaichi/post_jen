use anyhow::Result;
use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};

const SCHEMA_SQL: &str = include_str!("../../../db/schema.sql");

pub async fn connect(database_url: &str) -> Result<SqlitePool> {
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(database_url)
        .await?;

    sqlx::query("PRAGMA foreign_keys = ON;")
        .execute(&pool)
        .await?;

    for statement in split_sql_statements(SCHEMA_SQL) {
        sqlx::query(statement).execute(&pool).await?;
    }

    Ok(pool)
}

fn split_sql_statements(sql: &str) -> impl Iterator<Item = &str> {
    sql.split(';').map(str::trim).filter(|stmt| !stmt.is_empty())
}
