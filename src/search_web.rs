use serde_json::Value;

use crate::mcp;

#[derive(serde::Deserialize)]
struct SearXngResponse {
    results: Vec<SearXngResult>,
}

#[derive(serde::Deserialize)]
struct SearXngResult {
    content: String,
    title: String,
    url: String,
}

pub struct SearchConfig {
    num_results: usize,
    url: String,
}

#[derive(serde::Deserialize)]
struct SearchArgs {
    query: String,
    #[serde(default)]
    num_results: Option<usize>,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            num_results: 5,
            url: String::from("http://172.17.0.1:8888"),
        }
    }
}

impl SearchConfig {
    fn new() -> Self {
        let mut config = Self::default();
        if let Ok(url) = std::env::var("SEARXNG_URL") {
            config.url = url;
        }
        config
    }
}

fn format_result(r: &SearXngResult, i: usize) -> String {
    format!("{}. {}\n   {}\n   {}", i + 1, r.title, r.content, r.url)
}

pub async fn search(query: &str, config: &SearchConfig) -> Result<String, reqwest::Error> {
    let client = reqwest::Client::new();

    let resp: SearXngResponse = client
        .get(&format!("{}/search", config.url))
        .query(&[("q", query), ("format", "json")])
        .send()
        .await?
        .json()
        .await?;

    Ok(resp
        .results
        .iter()
        .enumerate()
        .map(|(i, r)| format_result(r, i))
        .take(config.num_results)
        .collect::<Vec<_>>()
        .join("\n\n"))
}

const DESCRIPTION: &str = r#"Search the web using SearXNG. Returns numbered results with
title, engine-generated snippet, and URL. Snippets are short; use
fetch_url on a URL to read the full page."#;

pub fn tool_definition() -> Value {
    serde_json::json!({
        "name": "search_web",
        "description": DESCRIPTION,
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": { "type": "string"},
                "num_results": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Maximum number of results to return (default 5).",
                }
            },
            "required": ["query"]
        }
    })
}

pub async fn handle_call(args: Value) -> Result<Value, mcp::JsonRpcErrorResponse> {
    let args: SearchArgs =
        serde_json::from_value(args).map_err(|_| mcp::invalid_params("invalid args"))?;

    let mut config = SearchConfig::new();
    if let Some(n) = args.num_results {
        config.num_results = n;
    }

    let text = search(&args.query, &config).await;

    match text {
        Ok(res) => Ok(serde_json::json!({
            "content": [
                { "type": "text", "text": res },
            ],
            "isError": false,
        })),
        Err(err) => Ok(serde_json::json!({
            "content": [
                { "type": "text", "text": err.to_string() },
            ],
            "isError": true,
        })),
    }
}
