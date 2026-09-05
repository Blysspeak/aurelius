use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use super::SearchResult;

const PERPLEXITY_API_URL: &str = "https://api.perplexity.ai/search";

#[derive(Serialize)]
struct PerplexityRequest<'a> {
    query: &'a str,
    max_results: usize,
    search_context_size: &'static str,
}

#[derive(Deserialize)]
struct PerplexityResponse {
    results: Vec<PerplexityResult>,
}

#[derive(Deserialize)]
struct PerplexityResult {
    title: String,
    url: String,
    snippet: Option<String>,
}

pub fn search(query: &str, count: usize) -> Result<Vec<SearchResult>> {
    let api_key = super::resolve_api_key("PERPLEXITY_API_KEY", "perplexity.key")?;
    let max_results = count.clamp(1, 20);

    let client = reqwest::blocking::Client::new();
    let resp = client
        .post(PERPLEXITY_API_URL)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .json(&PerplexityRequest {
            query,
            max_results,
            search_context_size: "medium",
        })
        .send()?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        bail!("Perplexity API error {status}: {body}");
    }

    let body = resp.text()?;
    parse_response(&body)
}

/// Parse a Perplexity `/search` response body into normalized results.
/// Kept separate from the HTTP call so it can be unit-tested without a
/// network round-trip.
fn parse_response(body: &str) -> Result<Vec<SearchResult>> {
    let data: PerplexityResponse = serde_json::from_str(body).map_err(|e| {
        anyhow::anyhow!(
            "Failed to parse Perplexity response: {e}\nBody: {}",
            &body[..body.len().min(500)]
        )
    })?;

    Ok(data
        .results
        .into_iter()
        .map(|r| SearchResult {
            title: r.title,
            url: r.url,
            description: r.snippet.unwrap_or_default(),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_response_maps_snippet_into_description() {
        let body = r#"{
            "id": "abc123",
            "results": [
                {
                    "title": "First result",
                    "url": "https://example.com/first",
                    "snippet": "a snippet",
                    "date": "2026-01-01",
                    "last_updated": "2026-01-02"
                },
                {
                    "title": "Second result",
                    "url": "https://example.com/second",
                    "date": null,
                    "last_updated": null
                }
            ],
            "server_time": null
        }"#;

        let results = parse_response(body).expect("fixture must parse");

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "First result");
        assert_eq!(results[0].url, "https://example.com/first");
        assert_eq!(results[0].description, "a snippet");
        assert_eq!(results[1].title, "Second result");
        assert_eq!(results[1].url, "https://example.com/second");
        assert_eq!(results[1].description, "");
    }
}
