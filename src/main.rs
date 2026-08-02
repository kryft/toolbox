mod man_page;
mod mcp;

use std::io::{self, BufRead, Write};

fn handle_request(
    req: mcp::JsonRpcRequest,
) -> Result<mcp::JsonRpcResponse, mcp::JsonRpcErrorResponse> {
    match req.method.as_str() {
        "initialize" => Ok(mcp::JsonRpcResponse {
            id: req.id,
            jsonrpc: "2.0".into(),
            result: serde_json::json!({
                "protocolVersion": "2025-11-25",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "toolbox", "version": "0.1.0"}
            }),
        }),
        _other => Err(mcp::JsonRpcErrorResponse {
            id: Some(req.id),
            jsonrpc: "2.0".into(),
            error: mcp::JsonRpcError {
                code: mcp::METHOD_NOT_FOUND,
                message: "method not found".into(),
                data: None,
            },
        }),
    }
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

        match parsed {
            Ok(message) => match message {
                mcp::JsonRpcMessage::Notification(_) => eprintln!("Got a notification"),
                mcp::JsonRpcMessage::Request(req) => {
                    let resp = handle_request(req);
                    let serialized = match resp {
                        Ok(r) => serde_json::to_string(&r).expect("failed to serialize response"),
                        Err(r) => serde_json::to_string(&r).expect("failed to serialize response"),
                    };

                    writeln!(out, "{}", serialized).expect("failed to write to stdout");
                }
            },
            Err(err) => {
                let resp = mcp::JsonRpcErrorResponse {
                    jsonrpc: "2.0".into(),
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
