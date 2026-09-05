use anyhow::{bail, Result};
use serde::Deserialize;

pub use super::SearchResult;

const BRAVE_API_URL: &str = "https://api.search.brave.com/res/v1/web/search";

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

pub fn search(query: &str, count: usize) -> Result<Vec<SearchResult>> {
    let api_key = super::resolve_api_key("BRAVE_API_KEY", "brave.key")?;

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
    let data: BraveResponse = serde_json::from_str(&body).map_err(|e| {
        anyhow::anyhow!(
            "Failed to parse Brave response: {e}\nBody: {}",
            &body[..body.len().min(500)]
        )
    })?;

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
