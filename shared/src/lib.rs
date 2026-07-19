use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationJob {
    pub grant_id: u64,
    pub milestone_id: u64,
    pub evidence_uri: String,
    // NOTE: was `u64`, but every call site (indexer/src/main.rs,
    // verifier/src/verifier.rs) already constructs this with
    // `chrono::Utc::now().naive_utc()`, which is a `NaiveDateTime`, not a
    // u64. Fixed to match actual usage rather than the other way around,
    // since NaiveDateTime is what both services want anyway.
    pub submitted_at: NaiveDateTime,
}

/// Structured metadata extracted from a validated, merged GitHub PR
/// submitted as milestone evidence. Populated when `evidence_uri` is a
/// `github.com/.../pull/N` link that the verifier confirmed is merged.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrEvidence {
    pub owner: String,
    pub repo: String,
    pub pr_number: u64,
    pub title: String,
    /// GitHub login of the PR author.
    pub author: String,
    /// RFC3339 timestamp as returned by the GitHub API, e.g.
    /// "2026-06-14T18:22:31Z".
    pub merged_at: String,
    /// Immutable evidence anchor — unlike the title/body, this can't be
    /// edited after merge.
    pub merge_commit_sha: Option<String>,
    pub html_url: String,
    pub additions: u64,
    pub deletions: u64,
    pub changed_files: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VerificationResult {
    Approved {
        checked_at: NaiveDateTime,
        /// Present when the evidence was a GitHub PR URL; `None` for
        /// other approved evidence kinds (ipfs://, ar://, data:, ...).
        pr_evidence: Option<PrEvidence>,
    },
    Rejected {
        reason: String,
        checked_at: NaiveDateTime,
    },
}

impl VerificationResult {
    pub fn status_label(&self) -> &'static str {
        match self {
            VerificationResult::Approved { .. } => "Approved",
            VerificationResult::Rejected { .. } => "Rejected",
        }
    }

    pub fn reason(&self) -> Option<&str> {
        match self {
            VerificationResult::Rejected { reason, .. } => Some(reason),
            _ => None,
        }
    }

    pub fn checked_at(&self) -> NaiveDateTime {
        match self {
            VerificationResult::Approved { checked_at, .. } => *checked_at,
            VerificationResult::Rejected { checked_at, .. } => *checked_at,
        }
    }

    pub fn pr_evidence(&self) -> Option<&PrEvidence> {
        match self {
            VerificationResult::Approved { pr_evidence, .. } => pr_evidence.as_ref(),
            VerificationResult::Rejected { .. } => None,
        }
    }
}

impl fmt::Display for VerificationResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VerificationResult::Approved { checked_at, pr_evidence } => {
                match pr_evidence {
                    Some(pr) => write!(
                        f,
                        "Approved at {} (PR {}/{}#{} by {})",
                        checked_at, pr.owner, pr.repo, pr.pr_number, pr.author
                    ),
                    None => write!(f, "Approved at {}", checked_at),
                }
            }
            VerificationResult::Rejected { reason, checked_at } => {
                write!(f, "Rejected at {}: {}", checked_at, reason)
            }
        }
    }
}

#[derive(Debug, Error)]
pub enum QueueError {
    #[error("channel closed")]
    ChannelClosed,
}

impl From<tokio::sync::mpsc::error::SendError<VerificationJob>> for QueueError {
    fn from(_: tokio::sync::mpsc::error::SendError<VerificationJob>) -> Self {
        QueueError::ChannelClosed
    }
}
