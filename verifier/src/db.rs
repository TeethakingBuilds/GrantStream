use anyhow::Result;
use grantstream_shared::VerificationResult;
use sqlx::SqlitePool;

pub async fn init_pool(database_url: &str) -> Result<SqlitePool> {
    let pool = SqlitePool::connect(database_url).await?;
    Ok(pool)
}

pub async fn upsert_verification(
    pool: &SqlitePool,
    grant_id: u64,
    milestone_id: u64,
    result: &VerificationResult,
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
