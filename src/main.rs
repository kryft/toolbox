mod man_page;
mod mcp;

use std::io::{self, BufRead, Write};

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

        match parsed {
            Ok(_) => {
                eprintln!("Got a message")
            }
            Err(err) => {
                let resp = mcp::JsonRpcErrorResponse {
                    jsonrpc: "string".into(),
                    id: None,
                    error: err,
                };

                let serialized =
                    serde_json::to_string(&resp).expect("failed to serialize error response");

                writeln!(out, "{}", serialized).expect("failed to write to stdout");
            }
        }
    }
}
