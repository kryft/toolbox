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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_request(id: Value, method: &str, params: Option<Value>) -> mcp::JsonRpcRequest {
        mcp::JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id,
            method: method.into(),
            params,
        }
    }

    #[test]
    fn initialize_returns_server_info() {
        let req = make_request(Value::from(1), "initialize", None);
        let resp = handle_request(req).unwrap();

        assert_eq!(resp.id, 1);
        assert_eq!(resp.result["protocolVersion"], "2025-11-25");
        assert_eq!(resp.result["serverInfo"]["name"], "toolbox");
        assert_eq!(resp.result["serverInfo"]["version"], "0.1.0");
        assert!(resp.result["capabilities"]["tools"].is_object());
    }

    #[test]
    fn tools_list_returns_man_page() {
        let req = make_request(Value::from(2), "tools/list", None);
        let resp = handle_request(req).unwrap();

        assert_eq!(resp.id, 2);
        let tools = &resp.result["tools"];
        assert!(tools.is_array());
        assert_eq!(tools.as_array().unwrap().len(), 1);
        assert_eq!(tools[0]["name"], "man_page");
    }

    #[test]
    fn tools_call_unknown_tool() {
        let req = make_request(
            Value::from(3),
            "tools/call",
            Some(serde_json::json!({
                "name": "nonexistent",
                "arguments": { "topic": "ls" }
            })),
        );
        let err = handle_request(req).unwrap_err();

        assert!(err.id.is_none());
        assert_eq!(err.error.code, mcp::INVALID_PARAMS);
    }

    #[test]
    fn tools_call_missing_params() {
        let req = make_request(Value::from(4), "tools/call", None);
        let err = handle_request(req).unwrap_err();

        assert_eq!(err.error.code, mcp::INVALID_PARAMS);
    }

    #[test]
    fn tools_call_man_page_success() {
        let req = make_request(
            Value::from(5),
            "tools/call",
            Some(serde_json::json!({
                "name": "man_page",
                "arguments": { "topic": "ls" }
            })),
        );
        let resp = handle_request(req).unwrap();

        assert_eq!(resp.id, 5);
        assert_eq!(resp.result["isError"], false);
        assert!(!resp.result["content"][0]["text"].as_str().unwrap().is_empty());
    }

    #[test]
    fn unknown_method() {
        let req = make_request(Value::from(6), "foo/bar", None);
        let err = handle_request(req).unwrap_err();

        assert_eq!(err.id, Some(Value::from(6)));
        assert_eq!(err.error.code, mcp::METHOD_NOT_FOUND);
    }
}
