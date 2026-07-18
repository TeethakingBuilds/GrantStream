use anyhow::Context;
use axum::{
    extract::{Json, State},
    http::StatusCode,
    routing::post,
    Router,
};
use grantstream_shared::VerificationJob;
use sqlx::SqlitePool;
use tokio::sync::mpsc;
use tracing;

mod verifier;
mod db;
mod github;

#[derive(Clone)]
struct AppState {
    pool: SqlitePool,
    job_tx: mpsc::Sender<VerificationJob>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let _ = dotenvy::dotenv();

    let db_url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:verifier.db".into());
    let pool = db::init_pool(&db_url).await?;

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .context("failed to run migrations")?;

    let (job_tx, mut job_rx) = mpsc::channel::<VerificationJob>(100);

    tokio::spawn(async move {
        while let Some(job) = job_rx.recv().await {
            tracing::info!(?job, "processing verification job");
            if let Err(e) = process_job(&pool, &job).await {
                tracing::error!(?e, ?job, "verification failed");
            }
        }
    });

    let state = AppState { pool, job_tx };

    let app = Router::new()
        .route("/verify", post(receive_verification))
        .route("/health", axum::routing::get(|| async { "ok" }))
        .with_state(state);

    let addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:8081".into());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("Verifier listening on {}", addr);

    axum::serve(listener, app).await?;
    Ok(())
}

async fn receive_verification(
    State(state): State<AppState>,
    Json(job): Json<VerificationJob>,
) -> std::result::Result<StatusCode, StatusCode> {
    state.job_tx.send(job).await.map_err(|_| {
        tracing::error!("job channel full or closed");
        StatusCode::SERVICE_UNAVAILABLE
    })?;
    Ok(StatusCode::ACCEPTED)
}

async fn process_job(pool: &SqlitePool, job: &VerificationJob) -> anyhow::Result<()> {
    let result = verifier::verify(job).await;
    db::upsert_verification(pool, job.grant_id, job.milestone_id, &result)
        .await?;
    tracing::info!(?job, result = %result, "verification complete");
    Ok(())
}
