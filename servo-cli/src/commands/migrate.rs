//! Migrate command

use anyhow::Result;
use servo_storage::PostgresStorage;

pub async fn execute(database_url: &str, direction: &str) -> Result<()> {
    tracing::info!("Running migrations {} on {}", direction, database_url);

    match direction {
        "up" => {
            let storage = PostgresStorage::new(database_url).await?;
            servo_storage::migrations::run_migrations(storage.pool()).await?;
            tracing::info!("Migrations applied successfully");
        }
        "down" => {
            // TODO: Implement down migrations
            tracing::warn!("Down migrations not yet implemented");
        }
        _ => {
            anyhow::bail!("Invalid migration direction: {}", direction);
        }
    }

    Ok(())
}
