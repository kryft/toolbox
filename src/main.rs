mod fetch_url;
mod man_page;
mod mcp;
mod server;

use std::io::{self, Write};
use tokio::io::AsyncBufReadExt;

fn write_response(out: &mut impl Write, value: &impl serde::Serialize) -> io::Result<()> {
    let line = serde_json::to_string(value)?;
    writeln!(out, "{}", line)
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();

    let reader = tokio::io::BufReader::new(stdin);
    let mut lines = reader.lines();

    loop {
        let line = match lines.next_line().await {
            Ok(Some(l)) => l,
            Ok(None) => break,
            Err(_) => break,
        };

        if line.is_empty() {
            break;
        }

        let parsed = mcp::parse_message(&line);

        let response = match parsed {
            Ok(message) => match message {
                mcp::JsonRpcMessage::Notification(_) => {
                    eprintln!("Got a notification");
                    continue;
                }
                mcp::JsonRpcMessage::Request(req) => server::handle_request(req).await,
            },
            Err(err) => Err(mcp::JsonRpcErrorResponse {
                jsonrpc: "2.0".into(),
                id: None,
                error: err,
            }),
        };

        let result = match response {
            Ok(r) => write_response(&mut out, &r),
            Err(r) => write_response(&mut out, &r),
        };

        if let Err(e) = result {
            eprintln!("failed to write response: {}", e);
            break;
        }
    }
}
