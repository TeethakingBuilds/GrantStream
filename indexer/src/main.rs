use anyhow::{Context, Result};
use ethers::{
    prelude::*,
    providers::{Provider, Ws},
};
use grantstream_shared::VerificationJob;
use sqlx::SqlitePool;
use std::sync::Arc;
use tokio::sync::mpsc;

mod db;

ethers::contract::abigen!(
    GrantStreamEscrow,
    r#"[
        event MilestoneSubmitted(uint256 indexed grantId, uint256 indexed milestoneId, string evidenceURI)
    ]"#
);

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let config = load_config()?;

    let db_pool = db::init_pool(&config.database_url)
        .await
        .context("failed to init db pool")?;

    sqlx::migrate!("./migrations")
        .run(&db_pool)
        .await
        .context("failed to run migrations")?;

    let (job_tx, mut job_rx) = mpsc::channel::<VerificationJob>(100);

    let verifier_db_pool = db_pool.clone();
    tokio::spawn(async move {
        let verifier_url = std::env::var("VERIFIER_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8081".to_string());

        loop {
            match job_rx.recv().await {
                Some(job) => {
                    if let Err(e) = submit_verification(&verifier_url, &job).await {
                        tracing::error!(?e, "verifier submit failed for job {:?}", job);
                    }
                }
                None => {
                    tracing::info!("job channel closed");
                    break;
                }
            }
        }
    });

    tracing::info!("Connecting to provider {}", config.rpc_url);
    let ws = Ws::connect(&config.rpc_url).await?;
    let provider = Provider::new(ws);

    let contract_address: Address = config.contract_address.parse()?;
    let contract = GrantStreamEscrow::new(contract_address, Arc::new(provider));

    backfill_historical_events(&contract, &db_pool, &job_tx, &config).await?;

    let event_filter = contract.events::<MilestoneSubmittedFilter>();
    let mut stream = event_filter.stream().await?;
    tracing::info!("Listening for MilestoneSubmitted events...");

    while let Some(Ok(event)) = stream.next().await {
        let data = event.data;
        let parsed = data;

        let job = VerificationJob {
            grant_id: parsed.grant_id.as_u64(),
            milestone_id: parsed.milestone_id.as_u64(),
            evidence_uri: parsed.evidence_uri,
            submitted_at: chrono::Utc::now().naive_utc(),
        };

        tracing::info!(?job, "enqueuing verification job");

        if let Err(e) = db::insert_pending_job(&db_pool, &job).await {
            tracing::error!(?e, "failed to insert pending job");
            continue;
        }

        if let Err(e) = job_tx.send(job).await {
            tracing::error!(?e, "job channel send failed");
        }
    }

    Ok(())
}

async fn submit_verification(
    verifier_url: &str,
    job: &VerificationJob,
) -> Result<()> {
    let client = reqwest::Client::new();
    let res = client
        .post(format!("{}/verify", verifier_url))
        .json(job)
        .send()
        .await?;

    let _ = res.error_for_status()?;
    Ok(())
}

async fn backfill_historical_events(
    contract: &GrantStreamEscrow<Provider<Ws>>,
    db_pool: &SqlitePool,
    job_tx: &mpsc::Sender<VerificationJob>,
    config: &Config,
) -> Result<()> {
    let Some(deployment_block) = config.deployment_block else {
        tracing::info!("DEPLOYMENT_BLOCK not set — skipping historical backfill");
        return Ok(());
    };

    let current_block = contract.client().get_block_number().await?.as_u64();

    let resume_from = match db::get_last_indexed_block(db_pool).await? {
        Some(last) => last + 1,
        None => deployment_block,
    };

    if resume_from > current_block {
        tracing::info!(
            "Indexer already caught up through block {current_block} — nothing to backfill"
        );
        return Ok(());
    }

    tracing::info!(
        "Backfilling MilestoneSubmitted events from block {resume_from} to {current_block}"
    );

    let mut from = resume_from;
    while from <= current_block {
        let to = std::cmp::min(
            from.saturating_add(config.backfill_chunk_size.saturating_sub(1)),
            current_block,
        );

        let events = contract
            .events::<MilestoneSubmittedFilter>()
            .from_block(from)
            .to_block(to)
            .query()
            .await
            .with_context(|| format!("failed to query events for blocks {from}..={to}"))?;

        let event_count = events.len();

        for parsed in events {
            let job = VerificationJob {
                grant_id: parsed.grant_id.as_u64(),
                milestone_id: parsed.milestone_id.as_u64(),
                evidence_uri: parsed.evidence_uri,
                submitted_at: chrono::Utc::now().naive_utc(),
            };

            tracing::info!(?job, "enqueuing backfilled verification job");

            if let Err(e) = db::insert_pending_job(db_pool, &job).await {
                tracing::error!(?e, "failed to insert backfilled pending job");
                continue;
            }

            if let Err(e) = job_tx.send(job).await {
                tracing::error!(?e, "job channel send failed during backfill");
            }
        }

        // Persist progress after each chunk (not just at the end) so a
        // crash/restart mid-backfill resumes from the last *completed*
        // chunk instead of re-scanning the whole range from scratch.
        db::set_last_indexed_block(db_pool, to).await?;
        tracing::info!("Backfilled blocks {from}..={to} ({event_count} events)");

        from = to + 1;
    }

    tracing::info!("Backfill complete, caught up to block {current_block}");
    Ok(())
}

#[derive(Debug, Clone)]
pub struct Config {
    pub rpc_url: String,
    pub contract_address: String,
    pub database_url: String,
    /// Block the contract was deployed at. If set, the indexer backfills
    /// all `MilestoneSubmitted` events from here (or from the last
    /// indexed block on restart) up to the current chain head before
    /// switching to the live subscription. If unset, backfill is
    /// skipped entirely — matches the old behavior for anyone not ready
    /// to opt in yet.
    pub deployment_block: Option<u64>,
    /// Max block range per `eth_getLogs` call during backfill. Most RPC
    /// providers cap this (commonly 2000-10000); keep it conservative by
    /// default since a too-large range just gets rejected by the node.
    pub backfill_chunk_size: u64,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let deployment_block = match std::env::var("DEPLOYMENT_BLOCK") {
            Ok(v) => Some(
                v.parse::<u64>()
                    .context("DEPLOYMENT_BLOCK must be a valid block number")?,
            ),
            Err(_) => None,
        };

        let backfill_chunk_size = std::env::var("BACKFILL_CHUNK_SIZE")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(2000);

        Ok(Self {
            rpc_url: std::env::var("RPC_URL")?,
            contract_address: std::env::var("CONTRACT_ADDRESS")?,
            database_url: std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:indexer.db".into()),
            deployment_block,
            backfill_chunk_size,
        })
    }
}

fn load_config() -> Result<Config> {
    let _ = dotenvy::dotenv();
    Config::from_env()
}
