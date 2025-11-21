//! Database migration support

use crate::Result;
use sqlx::PgPool;

/// Run all pending migrations
pub async fn run_migrations(pool: &PgPool) -> Result<()> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .map_err(|e| crate::Error::Migration(e.to_string()))?;

    Ok(())
}
