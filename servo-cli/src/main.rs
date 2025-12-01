//! Servo CLI tool

use clap::{Parser, Subcommand};

mod commands;
mod config;
mod polling;

#[derive(Parser)]
#[command(name = "servo")]
#[command(author, version, about = "Servo orchestration platform CLI", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Database URL (overrides config file and DATABASE_URL env var)
    #[arg(long)]
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

        /// Wait for execution to complete before returning
        #[arg(long)]
        wait: bool,

        /// Timeout in seconds when using --wait (default: 600)
        #[arg(long, default_value = "600")]
        timeout: u64,

        /// Poll interval in seconds when using --wait (default: 2)
        #[arg(long, default_value = "2")]
        poll_interval: u64,
    },

    /// Check workflow execution status
    Status {
        /// Execution ID to check
        execution_id: String,

        /// Tenant ID
        #[arg(long, env = "TENANT_ID")]
        tenant_id: String,
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

    /// Manage partition backfills
    Backfill {
        #[command(subcommand)]
        action: BackfillAction,
    },
}

#[derive(Subcommand)]
enum BackfillAction {
    /// Trigger a backfill for a specific partition or date range
    Start {
        /// Asset name to backfill
        asset: String,

        /// Single partition key to backfill (e.g., "2024-01-15")
        /// Use this OR --start/--end for a range
        #[arg(long, conflicts_with_all = ["start", "end"])]
        partition: Option<String>,

        /// Start date for range backfill (YYYY-MM-DD format)
        #[arg(long, requires = "end")]
        start: Option<String>,

        /// End date for range backfill (YYYY-MM-DD format, inclusive)
        #[arg(long, requires = "start")]
        end: Option<String>,

        /// Include upstream dependencies in backfill
        /// When enabled, all upstream assets will be backfilled first
        #[arg(long, default_value = "false")]
        include_upstream: bool,

        /// Maximum depth for upstream asset discovery
        /// 0 = direct dependencies only, 1 = include their dependencies, etc.
        /// Default is 1 (direct dependencies and their immediate parents)
        #[arg(long, default_value = "1")]
        max_upstream_depth: i32,

        /// SLA deadline for the backfill job (ISO 8601 format, e.g., "2024-01-16T00:00:00Z")
        /// When set, the job will be tracked for SLA compliance
        #[arg(long)]
        sla_deadline: Option<String>,

        /// Job priority for scheduling (-10 to 10, higher = more urgent, default: 0)
        /// Higher priority jobs are claimed by executors first
        #[arg(long, default_value = "0", value_parser = clap::value_parser!(i32).range(-10..=10))]
        priority: i32,
    },

    /// List backfill jobs
    List {
        /// Filter by status (pending, running, paused, completed, failed, cancelled)
        #[arg(long)]
        status: Option<String>,
    },

    /// Get status of a backfill job
    Status {
        /// Backfill job ID
        job_id: String,
    },

    /// Pause a running backfill job at the next partition boundary
    Pause {
        /// Backfill job ID to pause
        job_id: String,
    },

    /// Resume a paused backfill job from where it left off
    Resume {
        /// Backfill job ID to resume
        job_id: String,
    },

    /// Cancel a backfill job
    Cancel {
        /// Backfill job ID to cancel
        job_id: String,

        /// Reason for cancellation (optional)
        #[arg(long)]
        reason: Option<String>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Setup logging
    let log_level = if cli.verbose { "debug" } else { "info" };
    tracing_subscriber::fmt().with_env_filter(log_level).init();

    // Load configuration (defaults → file → env vars)
    let config = config::Config::load()?;

    // Resolve database URL: CLI arg > config (which already has env > file > default)
    let resolve_db_url =
        |cli_url: Option<String>| cli_url.unwrap_or_else(|| config.database_url.clone());

    // Execute command
    match cli.command {
        Commands::Init { database_url } => {
            commands::init::execute(&database_url).await?;
        }
        Commands::Migrate { direction } => {
            let database_url = resolve_db_url(cli.database_url.clone());
            commands::migrate::execute(&database_url, &direction).await?;
        }
        Commands::Deploy { workflow_file } => {
            commands::deploy::execute(&workflow_file).await?;
        }
        Commands::Run {
            workflow_name,
            params,
            wait,
            timeout,
            poll_interval,
        } => {
            let database_url = resolve_db_url(cli.database_url.clone());

            let status = commands::run::execute(
                &workflow_name,
                params.as_deref(),
                wait,
                timeout,
                poll_interval,
                &database_url,
            )
            .await?;

            // Convert execution status to exit code
            use commands::run::ExecutionStatus;
            match status {
                ExecutionStatus::Succeeded(_) => std::process::exit(0),
                ExecutionStatus::Failed(_)
                | ExecutionStatus::Timeout(_)
                | ExecutionStatus::Cancelled(_) => std::process::exit(1),
                ExecutionStatus::AsyncStarted(_) => {
                    // No exit - execution started async
                }
            }
        }
        Commands::Status {
            execution_id,
            tenant_id,
        } => {
            let database_url = resolve_db_url(cli.database_url.clone());

            commands::status::execute(&execution_id, &tenant_id, &database_url).await?;
        }
        Commands::Lineage {
            name,
            upstream,
            downstream,
        } => {
            commands::lineage::execute(&name, upstream, downstream).await?;
        }
        Commands::Backfill { action } => {
            let database_url = resolve_db_url(cli.database_url.clone());

            match action {
                BackfillAction::Start {
                    asset,
                    partition,
                    start,
                    end,
                    include_upstream,
                    max_upstream_depth,
                    sla_deadline,
                    priority,
                } => {
                    match (partition, start, end) {
                        (Some(p), None, None) => {
                            // Single partition mode
                            commands::backfill::execute_single_partition(
                                &asset,
                                &p,
                                include_upstream,
                                max_upstream_depth,
                                sla_deadline.as_deref(),
                                priority,
                                &database_url,
                            )
                            .await?;
                        }
                        (None, Some(s), Some(e)) => {
                            // Range mode
                            commands::backfill::execute_range_backfill(
                                &asset,
                                &s,
                                &e,
                                include_upstream,
                                max_upstream_depth,
                                sla_deadline.as_deref(),
                                priority,
                                &database_url,
                            )
                            .await?;
                        }
                        _ => {
                            anyhow::bail!(
                                "Must specify either --partition for single partition \
                                 or --start and --end for date range backfill"
                            );
                        }
                    }
                }
                BackfillAction::List { status } => {
                    commands::backfill::list_jobs(status.as_deref(), &database_url).await?;
                }
                BackfillAction::Status { job_id } => {
                    commands::backfill::get_status(&job_id, &database_url).await?;
                }
                BackfillAction::Pause { job_id } => {
                    commands::backfill::pause_job(&job_id, &database_url).await?;
                }
                BackfillAction::Resume { job_id } => {
                    commands::backfill::resume_job(&job_id, &database_url).await?;
                }
                BackfillAction::Cancel { job_id, reason } => {
                    commands::backfill::cancel_job(&job_id, reason.as_deref(), &database_url)
                        .await?;
                }
            }
        }
    }

    Ok(())
}
