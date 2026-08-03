mod man_page;
mod mcp;

use std::io::{self, BufRead, Write};

#[derive(serde::Deserialize)]
struct ManPageArgs {
    topic: String,
    section: Option<String>,
}

#[derive(serde::Deserialize)]
struct CallToolParams {
    name: String,
    arguments: ManPageArgs,
}

fn invalid_params(msg: &str) -> mcp::JsonRpcErrorResponse {
    mcp::JsonRpcErrorResponse {
        id: None,
        jsonrpc: "2.0".into(),
        error: mcp::JsonRpcError {
            code: mcp::INVALID_PARAMS,
            data: None,
            message: msg.into(),
        },
    }
}

fn handle_tool_call(
    req: mcp::JsonRpcRequest,
) -> Result<mcp::JsonRpcResponse, mcp::JsonRpcErrorResponse> {
    let params: CallToolParams =
        serde_json::from_value(req.params.unwrap_or(serde_json::Value::Null))
            .map_err(|_| invalid_params("bad params"))?;

    if params.name.as_str() != "man_page" {
        return Err(invalid_params("unknown tool"));
    }

    let topic = params.arguments.topic.as_str();
    let section = params.arguments.section.as_deref();

    let man_page_res =
        man_page::lookup_man_page(topic, section, &man_page::ManLookupConfig::default());

    let result = match man_page_res {
        Ok(res) => serde_json::json!({
            "content": [
                { "type": "text", "text": res.content },
            ],
                "isError": false
        }),
        Err(err) => serde_json::json!({
            "content": [
                { "type": "text", "text": err.to_string() },
            ],
                "isError": true
        }),
    };

    Ok(mcp::JsonRpcResponse {
        id: req.id,
        jsonrpc: "2.0".into(),
        result: result,
    })
}

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
        "tools/list" => Ok(mcp::JsonRpcResponse {
            id: req.id,
            jsonrpc: "2.0".into(),
            result: serde_json::json!({
                "tools": [{
                    "name": "man_page",
                    "description": "Look up a man page",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "topic": { "type": "string", "description": "topic (e.g. 'ls')"},
                            "section": { "type": "string", "description": "section (1-8, n, l or p)"}
                        },
                        "required": ["topic"]
                    }
                }]
            }),
        }),
        "tools/call" => handle_tool_call(req),
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
