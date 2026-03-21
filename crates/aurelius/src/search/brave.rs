use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

const BRAVE_API_URL: &str = "https://api.search.brave.com/res/v1/web/search";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub description: String,
}

#[derive(Deserialize)]
struct BraveResponse {
    web: Option<BraveWeb>,
}

#[derive(Deserialize)]
struct BraveWeb {
    results: Vec<BraveResult>,
}

#[derive(Deserialize)]
struct BraveResult {
    title: String,
    url: String,
    description: Option<String>,
}

fn resolve_api_key() -> Result<String> {
    if let Ok(key) = std::env::var("BRAVE_API_KEY") {
        return Ok(key);
    }
    // Fallback: read from config file
    let config_path = dirs_next::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join("aurelius")
        .join("brave.key");
    if config_path.exists() {
        let key = std::fs::read_to_string(&config_path)?.trim().to_owned();
        if !key.is_empty() {
            return Ok(key);
        }
    }
    anyhow::bail!("BRAVE_API_KEY not set and ~/.config/aurelius/brave.key not found")
}

pub fn search(query: &str, count: usize) -> Result<Vec<SearchResult>> {
    let api_key = resolve_api_key()?;

    let client = reqwest::blocking::Client::new();
    let resp = client
        .get(BRAVE_API_URL)
        .header("Accept", "application/json")
        .header("X-Subscription-Token", &api_key)
        .query(&[("q", query), ("count", &count.to_string())])
        .send()?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        bail!("Brave API error {status}: {body}");
    }

    let body = resp.text()?;
    let data: BraveResponse = serde_json::from_str(&body)
        .map_err(|e| anyhow::anyhow!("Failed to parse Brave response: {e}\nBody: {}", &body[..body.len().min(500)]))?;

    let results = data
        .web
        .map(|w| {
            w.results
                .into_iter()
                .map(|r| SearchResult {
                    title: r.title,
                    url: r.url,
                    description: r.description.unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(results)
}
