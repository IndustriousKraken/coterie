use anyhow::{Context, Result};

const USER_AGENT: &str = concat!("coterie-provision/", env!("CARGO_PKG_VERSION"));

/// Fetch the most recent ~10 releases from the GitHub API. Returns the
/// raw JSON body as a string so callers can pass it to
/// [`crate::version_selector::parse_releases`]. We keep parsing and
/// fetching separated so tests can feed a fixture without going over
/// the network.
pub fn fetch_releases(repo: &str) -> Result<String> {
    let url = format!("https://api.github.com/repos/{repo}/releases?per_page=10");
    let client = reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .context("failed to build HTTP client")?;
    let resp = client
        .get(&url)
        .send()
        .with_context(|| format!("failed to GET {url}"))?;
    let resp = resp
        .error_for_status()
        .with_context(|| format!("GET {url} returned non-2xx"))?;
    resp.text()
        .with_context(|| format!("failed to read body of {url}"))
}
