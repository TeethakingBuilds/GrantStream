use crate::github;
use grantstream_shared::VerificationJob;
use grantstream_shared::VerificationResult;

pub async fn verify(job: &VerificationJob) -> VerificationResult {
    let now = chrono::Utc::now().naive_utc();

    if job.evidence_uri.is_empty() {
        return VerificationResult::Rejected {
            reason: "Evidence URI is empty or malformed".to_string(),
            checked_at: now,
        };
    }

    // GitHub PR evidence gets a stronger check than the generic
    // scheme-prefix validation below: we actually confirm the PR exists,
    // is merged, and (optionally) belongs to the expected repo, and pull
    // back structured metadata to store with the milestone.
    if github::is_github_pr_url(&job.evidence_uri) {
        return match github::verify_github_pr(&job.evidence_uri).await {
            Ok(pr_evidence) => VerificationResult::Approved {
                checked_at: now,
                pr_evidence: Some(pr_evidence),
            },
            Err(e) => VerificationResult::Rejected {
                reason: e.to_string(),
                checked_at: now,
            },
        };
    }

    if validate_evidence(&job.evidence_uri) {
        VerificationResult::Approved {
            checked_at: now,
            pr_evidence: None,
        }
    } else {
        VerificationResult::Rejected {
            reason: "Evidence URI is empty or malformed".to_string(),
            checked_at: now,
        }
    }
}

fn validate_evidence(uri: &str) -> bool {
    if uri.is_empty() {
        return false;
    }

    uri.starts_with("ipfs://")
        || uri.starts_with("https://")
        || uri.starts_with("http://")
        || uri.starts_with("ar://")
        || uri.starts_with("data:")
}
