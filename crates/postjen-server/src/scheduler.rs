use anyhow::Result;
use chrono::Utc;
use cron::Schedule;
use sqlx::{FromRow, SqlitePool};
use std::str::FromStr;
use tokio::time::{Duration, interval, sleep};
use tracing::{error, info, warn};

pub fn spawn(pool: SqlitePool) {
    tokio::spawn(async move {
        // Check every 30 seconds
        let mut ticker = interval(Duration::from_secs(30));
        loop {
            ticker.tick().await;
            if let Err(error) = check_cron_triggers(&pool).await {
                error!(?error, "cron scheduler check failed");
                sleep(Duration::from_secs(5)).await;
            }
        }
    });
}

#[derive(Debug, FromRow)]
struct CronJob {
    id: i64,
    job_id: String,
    name: String,
    definition_path: String,
    definition_hash: String,
    triggers_json: Option<String>,
    last_triggered_at: Option<String>,
}

async fn check_cron_triggers(pool: &SqlitePool) -> Result<()> {
    let jobs = sqlx::query_as::<_, CronJob>(
        r#"
        SELECT id, job_id, name, definition_path, definition_hash, triggers_json, last_triggered_at
        FROM job_definitions
        WHERE enabled = 1
          AND triggers_json IS NOT NULL
        "#,
    )
    .fetch_all(pool)
    .await?;

    let now = Utc::now();

    for job in &jobs {
        let triggers_json = match &job.triggers_json {
            Some(json) => json,
            None => continue,
        };

        let triggers: serde_json::Value = match serde_json::from_str(triggers_json) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let cron_expr = match triggers.get("cron").and_then(|v| v.as_str()) {
            Some(expr) => expr,
            None => continue,
        };

        let schedule = match Schedule::from_str(cron_expr) {
            Ok(s) => s,
            Err(e) => {
                warn!(job_id = %job.job_id, cron = %cron_expr, ?e, "invalid cron expression");
                continue;
            }
        };

        // Find the most recent scheduled time
        let last_triggered = job.last_triggered_at.as_ref().and_then(|s| {
            chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
                .ok()
                .map(|dt| dt.and_utc())
        });

        // Check if there's a scheduled time between last_triggered and now
        let should_trigger = if let Some(last) = last_triggered {
            schedule.after(&last).take(1).any(|next| next < now)
        } else {
            // Never triggered before - check if current time matches
            schedule.after(&(now - chrono::Duration::seconds(60))).take(1).any(|next| next < now)
        };

        if should_trigger {
            info!(job_id = %job.job_id, "cron trigger fired");

            // Load definition to get working_dir
            let definition = match postjen_core::definition::JobDefinition::load(&job.definition_path) {
                Ok(d) => d,
                Err(e) => {
                    error!(job_id = %job.job_id, ?e, "failed to load definition for cron trigger");
                    continue;
                }
            };

            let mut tx = pool.begin().await?;

            let result = sqlx::query(
                r#"
                INSERT INTO job_runs (
                    job_definition_id, job_id, job_name, status, trigger_type, triggered_by,
                    definition_path, definition_hash, working_dir, queued_at
                )
                VALUES (?, ?, ?, 'queued', 'cron', 'scheduler', ?, ?, ?, CURRENT_TIMESTAMP)
                "#,
            )
            .bind(job.id)
            .bind(&job.job_id)
            .bind(&job.name)
            .bind(&job.definition_path)
            .bind(&job.definition_hash)
            .bind(&definition.working_dir)
            .execute(&mut *tx)
            .await?;

            let run_id = result.last_insert_rowid();

            sqlx::query(
                "INSERT INTO run_events (job_run_id, node_run_id, scope, event_type, from_status, to_status, message, occurred_at) VALUES (?, NULL, 'job', 'status_changed', NULL, 'queued', 'cron triggered', CURRENT_TIMESTAMP)"
            )
            .bind(run_id)
            .execute(&mut *tx)
            .await?;

            sqlx::query("UPDATE job_definitions SET last_triggered_at = ? WHERE id = ?")
                .bind(now.format("%Y-%m-%d %H:%M:%S").to_string())
                .bind(job.id)
                .execute(&mut *tx)
                .await?;

            tx.commit().await?;
        }
    }

    Ok(())
}
