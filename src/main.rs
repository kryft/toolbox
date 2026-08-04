mod man_page;
mod mcp;
mod server;

use std::io::{self, BufRead, Write};

fn write_response(
    out: &mut impl Write,
    value: &impl serde::Serialize,
) -> io::Result<()> {
    let line = serde_json::to_string(value)?;
    writeln!(out, "{}", line)
}

fn main() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();

    for line in stdin.lock().lines() {
        let message = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if message.is_empty() {
            break;
        }

        let parsed = mcp::parse_message(&message);

        let response = match parsed {
            Ok(message) => match message {
                mcp::JsonRpcMessage::Notification(_) => {
                    eprintln!("Got a notification");
                    continue;
                }
                mcp::JsonRpcMessage::Request(req) => server::handle_request(req),
            },
            Err(err) => {
                Err(mcp::JsonRpcErrorResponse {
                    jsonrpc: "2.0".into(),
                    id: None,
                    error: err,
                })
            }
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
