//! Validates GitHub PR URLs submitted as milestone evidence.
//!
//! A grantee can submit `evidence_uri = "https://github.com/{owner}/{repo}/pull/{n}"`
//! instead of an IPFS/Arweave URI. When that's detected, we hit the GitHub
//! REST API to confirm the PR is real and merged, and extract the metadata
//! that gets persisted alongside the milestone (see `db::upsert_verification`).

use grantstream_shared::PrEvidence;
use reqwest::{header, StatusCode};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GithubVerifyError {
    #[error("not a GitHub PR URL")]
    NotAPrUrl,

    #[error("PR is on {actual_owner}/{actual_repo}, but this program only accepts evidence from {expected_owner}/{expected_repo}")]
    RepoNotAllowed {
        expected_owner: String,
        expected_repo: String,
        actual_owner: String,
        actual_repo: String,
    },

    #[error("PR #{0} does not exist or is not accessible")]
    NotFound(u64),

    #[error("PR #{0} exists but is not merged (state: {1})")]
    NotMerged(u64, String),

    #[error("GitHub API rate limit exceeded (resets at {0})")]
    RateLimited(String),

    #[error("GitHub API returned status {0}: {1}")]
    UnexpectedStatus(StatusCode, String),

    #[error("network error contacting GitHub API: {0}")]
    Network(#[from] reqwest::Error),

    #[error("could not parse GitHub API response: {0}")]
    Parse(String),
}

/// True if `uri` looks like a `github.com/.../pull/N` link. Cheap check
/// used by `verifier::verify` to decide whether to route into this module.
pub fn is_github_pr_url(uri: &str) -> bool {
    parse_pr_url(uri).is_ok()
}

fn parse_pr_url(uri: &str) -> Result<(String, String, u64), GithubVerifyError> {
    let parsed = url::Url::parse(uri.trim()).map_err(|_| GithubVerifyError::NotAPrUrl)?;

    let host = parsed.host_str().unwrap_or_default();
    if host != "github.com" && host != "www.github.com" {
        return Err(GithubVerifyError::NotAPrUrl);
    }

    let segments: Vec<&str> = parsed
        .path_segments()
        .map(|s| s.filter(|seg| !seg.is_empty()).collect())
        .unwrap_or_default();

    if segments.len() < 4 || segments[2] != "pull" {
        return Err(GithubVerifyError::NotAPrUrl);
    }

    let owner = segments[0].to_string();
    let repo = segments[1].to_string();
    let pr_number: u64 = segments[3].parse().map_err(|_| GithubVerifyError::NotAPrUrl)?;

    Ok((owner, repo, pr_number))
}

#[derive(Debug, serde::Deserialize)]
struct GhPullResponse {
    title: String,
    state: String,
    merged: bool,
    merged_at: Option<String>,
    merge_commit_sha: Option<String>,
    html_url: String,
    additions: Option<u64>,
    deletions: Option<u64>,
    changed_files: Option<u64>,
    user: GhUser,
    base: GhRefTarget,
}

#[derive(Debug, serde::Deserialize)]
struct GhUser {
    login: String,
}

#[derive(Debug, serde::Deserialize)]
struct GhRefTarget {
    repo: GhRepo,
}

#[derive(Debug, serde::Deserialize)]
struct GhRepo {
    name: String,
    owner: GhUser,
}

/// Fetches and validates the PR at `evidence_uri`, returning structured
/// metadata on success.
///
/// If `GITHUB_EXPECTED_OWNER` / `GITHUB_EXPECTED_REPO` env vars are set,
/// the PR's base repo must match them — use this to restrict a given
/// verifier deployment to only accept evidence from one project's repo.
/// If unset, any repo is accepted (the PR just has to exist and be merged).
///
/// Reads `GITHUB_TOKEN` from the environment if present. Unauthenticated
/// requests are capped at 60/hr per IP by GitHub; set a token (a
/// fine-grained PAT with read-only public repo access is enough) to raise
/// that to 5000/hr, which matters once job volume picks up.
pub async fn verify_github_pr(evidence_uri: &str) -> Result<PrEvidence, GithubVerifyError> {
    let (owner, repo, pr_number) = parse_pr_url(evidence_uri)?;

    if let (Ok(expected_owner), Ok(expected_repo)) = (
        std::env::var("GITHUB_EXPECTED_OWNER"),
        std::env::var("GITHUB_EXPECTED_REPO"),
    ) {
        if !owner.eq_ignore_ascii_case(&expected_owner) || !repo.eq_ignore_ascii_case(&expected_repo) {
            return Err(GithubVerifyError::RepoNotAllowed {
                expected_owner,
                expected_repo,
                actual_owner: owner,
                actual_repo: repo,
            });
        }
    }

    let api_url = format!(
        "https://api.github.com/repos/{}/{}/pulls/{}",
        owner, repo, pr_number
    );

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .user_agent("grantstream-verifier")
        .build()?;

    let mut req = client
        .get(&api_url)
        .header(header::ACCEPT, "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28");

    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        req = req.header(header::AUTHORIZATION, format!("Bearer {}", token));
    }

    let resp = req.send().await?;

    match resp.status() {
        StatusCode::OK => {}
        StatusCode::NOT_FOUND => return Err(GithubVerifyError::NotFound(pr_number)),
        StatusCode::FORBIDDEN => {
            let reset = resp
                .headers()
                .get("x-ratelimit-reset")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("unknown")
                .to_string();
            return Err(GithubVerifyError::RateLimited(reset));
        }
        other => {
            let body = resp.text().await.unwrap_or_default();
            return Err(GithubVerifyError::UnexpectedStatus(other, body));
        }
    }

    let body: GhPullResponse = resp
        .json()
        .await
        .map_err(|e| GithubVerifyError::Parse(e.to_string()))?;

    // Defense in depth: re-check the base repo against what the URL
    // claimed, in case of a repo rename/transfer between submission and
    // verification, then again against the expected-repo env config.
    if let (Ok(expected_owner), Ok(expected_repo)) = (
        std::env::var("GITHUB_EXPECTED_OWNER"),
        std::env::var("GITHUB_EXPECTED_REPO"),
    ) {
        if !body.base.repo.owner.login.eq_ignore_ascii_case(&expected_owner)
            || !body.base.repo.name.eq_ignore_ascii_case(&expected_repo)
        {
            return Err(GithubVerifyError::RepoNotAllowed {
                expected_owner,
                expected_repo,
                actual_owner: body.base.repo.owner.login,
                actual_repo: body.base.repo.name,
            });
        }
    }

    if !body.merged {
        return Err(GithubVerifyError::NotMerged(pr_number, body.state));
    }

    let merged_at = body
        .merged_at
        .ok_or_else(|| GithubVerifyError::Parse("merged=true but merged_at was null".to_string()))?;

    Ok(PrEvidence {
        owner,
        repo,
        pr_number,
        title: body.title,
        author: body.user.login,
        merged_at,
        merge_commit_sha: body.merge_commit_sha,
        html_url: body.html_url,
        additions: body.additions.unwrap_or(0),
        deletions: body.deletions.unwrap_or(0),
        changed_files: body.changed_files.unwrap_or(0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_pr_url() {
        assert!(is_github_pr_url(
            "https://github.com/Sycosmile/grantstream-verifier/pull/628"
        ));
        assert!(is_github_pr_url("https://github.com/foo/bar/pull/1/files"));
    }

    #[test]
    fn rejects_non_pr_evidence() {
        assert!(!is_github_pr_url("ipfs://QmSomeHash"));
        assert!(!is_github_pr_url("https://github.com/foo/bar/issues/1"));
        assert!(!is_github_pr_url("https://gitlab.com/foo/bar/pull/1"));
        assert!(!is_github_pr_url("data:text/plain;base64,SGVsbG8="));
    }

    #[test]
    fn parses_owner_repo_number() {
        let (owner, repo, n) = parse_pr_url("https://github.com/foo/bar/pull/42").unwrap();
        assert_eq!((owner.as_str(), repo.as_str(), n), ("foo", "bar", 42));
    }

    // verify_github_pr itself needs a live/mocked HTTP call to cover the
    // NotFound/NotMerged/RepoNotAllowed/RateLimited branches — wire up
    // `wiremock` against a mocked api.github.com base if you want that
    // in CI rather than hitting the real API.
}
