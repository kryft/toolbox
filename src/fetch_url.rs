use serde_json::Value;

use crate::mcp;

pub fn tool_definition() -> Value {
    serde_json::json!({
        "name": "fetch_url",
        "description": "Fetch a URL and return its text content",
        "inputSchema": {
            "type": "object",
            "properties": {
                "url": { "type": "string"}
            },
            "required": ["url"]
        }
    })
}

async fn fetch(url: &str) -> Result<String, reqwest::Error> {
    let resp = reqwest::get(url).await?;
    return resp.text().await;
}

pub async fn handle_call(args: Value) -> Result<Value, mcp::JsonRpcErrorResponse> {
    let url: &str = args["url"].as_str().unwrap_or("");

    let text = fetch(url).await;

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
        assert_eq!(result["content"][0]["text"], "hello");

        server.await.unwrap();
    }
}
