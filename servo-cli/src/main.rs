//! Servo CLI tool

use clap::{Parser, Subcommand};

mod commands;
mod config;

#[derive(Parser)]
#[command(name = "servo")]
#[command(author, version, about = "Servo orchestration platform CLI", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Database URL
    #[arg(long, env = "DATABASE_URL")]
    database_url: Option<String>,

    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize Servo metadata database
    Init {
        /// Database URL to initialize
        #[arg(long)]
        database_url: String,
    },

    /// Run database migrations
    Migrate {
        /// Migration direction (up or down)
        #[arg(default_value = "up")]
        direction: String,
    },

    /// Deploy a workflow
    Deploy {
        /// Path to workflow file
        workflow_file: String,
    },

    /// Run a workflow
    Run {
        /// Workflow name to run
        workflow_name: String,

        /// Execution parameters (JSON)
        #[arg(long)]
        params: Option<String>,
    },

    /// Check workflow execution status
    Status {
        /// Execution ID to check
        execution_id: String,
    },

    /// Show workflow lineage
    Lineage {
        /// Asset or workflow name
        name: String,

        /// Show upstream dependencies
        #[arg(long)]
        upstream: bool,

        /// Show downstream dependencies
        #[arg(long)]
        downstream: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Setup logging
    let log_level = if cli.verbose { "debug" } else { "info" };
    tracing_subscriber::fmt()
        .with_env_filter(log_level)
        .init();

    // Execute command
    match cli.command {
        Commands::Init { database_url } => {
            commands::init::execute(&database_url).await?;
        }
        Commands::Migrate { direction } => {
            let database_url = cli
                .database_url
                .ok_or_else(|| anyhow::anyhow!("DATABASE_URL not set"))?;
            commands::migrate::execute(&database_url, &direction).await?;
        }
        Commands::Deploy { workflow_file } => {
            commands::deploy::execute(&workflow_file).await?;
        }
        Commands::Run {
            workflow_name,
            params,
        } => {
            commands::run::execute(&workflow_name, params.as_deref()).await?;
        }
        Commands::Status { execution_id } => {
            commands::status::execute(&execution_id).await?;
        }
        Commands::Lineage {
            name,
            upstream,
            downstream,
        } => {
            commands::lineage::execute(&name, upstream, downstream).await?;
        }
    }

    Ok(())
}
