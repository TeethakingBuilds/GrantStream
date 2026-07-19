use anyhow::Result;
use grantstream_shared::VerificationJob;
use sqlx::SqlitePool;

pub async fn init_pool(database_url: &str) -> Result<SqlitePool> {
    let pool = SqlitePool::connect(database_url).await?;
    Ok(pool)
}

/// Returns the last block the indexer confirmed it fully processed, or
/// `None` if this is a fresh DB (never backfilled or indexed anything yet).
pub async fn get_last_indexed_block(pool: &SqlitePool) -> Result<Option<u64>> {
    let row: Option<(i64,)> =
        sqlx::query_as("SELECT last_indexed_block FROM indexer_state WHERE id = 1")
            .fetch_optional(pool)
            .await?;

    Ok(row.map(|(b,)| b as u64))
}

/// Records `block` as the last block fully processed. Called after each
/// backfill chunk completes so a restart mid-backfill resumes from here
/// rather than re-scanning from the deployment block.
pub async fn set_last_indexed_block(pool: &SqlitePool, block: u64) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO indexer_state (id, last_indexed_block)
        VALUES (1, ?)
        ON CONFLICT(id) DO UPDATE SET last_indexed_block = excluded.last_indexed_block
        "#,
    )
    .bind(block as i64)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn insert_pending_job(pool: &SqlitePool, job: &VerificationJob) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO milestone_verifications (grant_id, milestone_id, evidence_uri, submitted_at, status)
        VALUES (?, ?, ?, ?, 'Pending')
        ON CONFLICT(grant_id, milestone_id) DO UPDATE SET
            evidence_uri = excluded.evidence_uri,
            submitted_at = excluded.submitted_at,
            status = 'Pending'
        "#,
    )
    .bind(job.grant_id as i64)
    .bind(job.milestone_id as i64)
    .bind(&job.evidence_uri)
    .bind(job.submitted_at.to_string())
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn upsert_verification(
    pool: &SqlitePool,
    grant_id: u64,
    milestone_id: u64,
    result: &grantstream_shared::VerificationResult,
) -> Result<()> {
    let pr = result.pr_evidence();

    sqlx::query(
        r#"
        UPDATE milestone_verifications
        SET status = ?, result_reason = ?, verified_at = ?,
            pr_owner = ?, pr_repo = ?, pr_number = ?, pr_title = ?,
            pr_author = ?, pr_merged_at = ?, pr_merge_commit_sha = ?,
            pr_html_url = ?, pr_additions = ?, pr_deletions = ?, pr_changed_files = ?
        WHERE grant_id = ? AND milestone_id = ?
        "#,
    )
    .bind(result.status_label())
    .bind(result.reason())
    .bind(result.checked_at().to_string())
    .bind(pr.map(|p| p.owner.clone()))
    .bind(pr.map(|p| p.repo.clone()))
    .bind(pr.map(|p| p.pr_number as i64))
    .bind(pr.map(|p| p.title.clone()))
    .bind(pr.map(|p| p.author.clone()))
    .bind(pr.map(|p| p.merged_at.clone()))
    .bind(pr.and_then(|p| p.merge_commit_sha.clone()))
    .bind(pr.map(|p| p.html_url.clone()))
    .bind(pr.map(|p| p.additions as i64))
    .bind(pr.map(|p| p.deletions as i64))
    .bind(pr.map(|p| p.changed_files as i64))
    .bind(grant_id as i64)
    .bind(milestone_id as i64)
    .execute(pool)
    .await?;

    Ok(())
}
