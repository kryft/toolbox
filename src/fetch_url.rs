use serde_json::Value;

use crate::mcp;

const INLINE_LIMIT: usize = 32 * 1024;
const PREVIEW_BYTES: usize = 2048;

const DESCRIPTION: &str = r#"Fetch a URL and store it as a document.
Responses under 32 KB are returned inline as text; larger responses
return a document id and a short preview. Narrow stored documents with
search_doc (exact substring), read_doc (byte windows), triage_doc
(LLM-ranked relevance), or summarize_doc (overview + structure map).
Large documents: fetch_url -> summarize_doc for a rough overview ->
triage_doc or re-summarize the spans you care about -> read_doc for
verbatim detail."#;

pub fn tool_definition() -> Value {
    serde_json::json!({
        "name": "fetch_url",
        "description": DESCRIPTION,
        "inputSchema": {
            "type": "object",
            "properties": {
                "url": { "type": "string"}
            },
            "required": ["url"]
        }
    })
}

async fn fetch(url: &str) -> Result<(String, Vec<u8>), reqwest::Error> {
    let resp = reqwest::get(url).await?;
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string();
    let body = resp.bytes().await?.to_vec();
    Ok((content_type, body))
}

async fn build_response(url: &str) -> Result<String, String> {
    let (content_type, body) = fetch(url).await.map_err(|e| e.to_string())?;
    let id = crate::store::save(url, &content_type, &body).map_err(|e| e.to_string())?;

    Ok(if body.len() < INLINE_LIMIT {
        format!(
            "[stored: {} ({} bytes, {})]\n{}",
            id,
            body.len(),
            content_type,
            String::from_utf8_lossy(&body)
        )
    } else {
        format!(
            "Document stored: {} ({} bytes, {})\nurl: {}\nPreview (first {} bytes):\n{}",
            id,
            body.len(),
            content_type,
            url,
            PREVIEW_BYTES,
            String::from_utf8_lossy(&body[..PREVIEW_BYTES])
        )
    })
}

pub async fn handle_call(args: Value) -> Result<Value, mcp::JsonRpcErrorResponse> {
    let url: &str = args["url"].as_str().unwrap_or("");

    match build_response(url).await {
        Ok(res) => Ok(serde_json::json!({
            "content": [{"type": "text", "text": res}],
            "isError": false
        })),
        Err(err) => Ok(serde_json::json!({
            "content": [{"type": "text", "text": err}],
            "isError": true
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn fetch_url_success() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{}", addr);

        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();

            let mut buf = [0u8; 1024];
            let _n = socket.read(&mut buf).await.unwrap();

            let response = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello";
            socket.write_all(response).await.unwrap();
        });

        let args = serde_json::json!({ "url": url });
        let result = handle_call(args).await.unwrap();

        assert_eq!(result["isError"], false);
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.starts_with("[stored: "));
        assert!(text.ends_with("\nhello"));

        server.await.unwrap();
    }

    #[tokio::test]
    async fn fetch_url_large_is_preview() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{}", addr);

        let body = "x".repeat(40 * 1024);
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();

            let mut buf = [0u8; 1024];
            let _n = socket.read(&mut buf).await.unwrap();

            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        });

        let args = serde_json::json!({ "url": url });
        let result = handle_call(args).await.unwrap();

        assert_eq!(result["isError"], false);
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.starts_with("Document stored: "));
        assert!(text.contains("Preview (first 2048 bytes):"));
        assert!(text.len() < 4096, "full body must not be inlined");

        server.await.unwrap();
    }
}
