use serde_json::Value;

use crate::man_page;
use crate::mcp;

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

pub fn handle_request(
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
                "tools": [man_page::tool_definition()]
            }),
        }),
        "tools/call" => {
            let params: CallToolParams =
                serde_json::from_value(req.params.unwrap_or(Value::Null))
                    .map_err(|_| invalid_params("bad params"))?;

            if params.name.as_str() != "man_page" {
                return Err(invalid_params("unknown tool"));
            }

            let args = serde_json::json!({
                "topic": params.arguments.topic,
                "section": params.arguments.section,
            });
            let result = man_page::handle_call(args)?;

            Ok(mcp::JsonRpcResponse {
                id: req.id,
                jsonrpc: "2.0".into(),
                result,
            })
        }
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
