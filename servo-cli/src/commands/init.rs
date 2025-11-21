//! Initialize command

use anyhow::Result;
use servo_storage::PostgresStorage;

pub async fn execute(database_url: &str) -> Result<()> {
    tracing::info!("Initializing Servo database at {}", database_url);

    // Create storage instance
    let storage = PostgresStorage::new(database_url).await?;

    // Run migrations
    servo_storage::migrations::run_migrations(storage.pool()).await?;

    tracing::info!("✅ Database initialized successfully");

    Ok(())
}
