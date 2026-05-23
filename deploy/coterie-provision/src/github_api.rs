use anyhow::{Context, Result};

const USER_AGENT: &str = "coterie-provision";
const RELEASES_URL: &str = "https://api.github.com/repos/IndustriousKraken/coterie/releases";

/// Fetch the most recent 10 releases for IndustriousKraken/coterie.
///
/// GitHub's `/releases` endpoint returns up to 100 per page; we cap at
/// 10 because the wizard only ever shows ~5 + "see all".
pub fn fetch_recent_releases() -> Result<String> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .context("failed to build HTTP client")?;
    let response = client
        .get(format!("{RELEASES_URL}?per_page=10"))
        .send()
        .context("failed to reach api.github.com")?;
    let status = response.status();
    let body = response
        .text()
        .context("failed to read GitHub API response body")?;
    if !status.is_success() {
        anyhow::bail!("github releases API returned {status}: {body}");
    }
    Ok(body)
}
