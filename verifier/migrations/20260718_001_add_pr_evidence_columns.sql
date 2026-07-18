-- PR metadata for milestone evidence submitted as a GitHub PR URL.
-- Populated by verifier::github::verify_github_pr when evidence_uri
-- matches https://github.com/{owner}/{repo}/pull/{n} and the PR is
-- confirmed merged. Left NULL for non-GitHub evidence (ipfs://, ar://,
-- data:, plain https URLs to other sites, etc.)
ALTER TABLE milestone_verifications ADD COLUMN pr_owner TEXT;
ALTER TABLE milestone_verifications ADD COLUMN pr_repo TEXT;
ALTER TABLE milestone_verifications ADD COLUMN pr_number INTEGER;
ALTER TABLE milestone_verifications ADD COLUMN pr_title TEXT;
ALTER TABLE milestone_verifications ADD COLUMN pr_author TEXT;
ALTER TABLE milestone_verifications ADD COLUMN pr_merged_at TEXT;
ALTER TABLE milestone_verifications ADD COLUMN pr_merge_commit_sha TEXT;
ALTER TABLE milestone_verifications ADD COLUMN pr_html_url TEXT;
ALTER TABLE milestone_verifications ADD COLUMN pr_additions INTEGER;
ALTER TABLE milestone_verifications ADD COLUMN pr_deletions INTEGER;
ALTER TABLE milestone_verifications ADD COLUMN pr_changed_files INTEGER;
