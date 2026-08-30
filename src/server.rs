use serde_json::Value;

use crate::fetch_url;
use crate::man_page;
use crate::mcp;
use crate::read_doc;
use crate::search_doc;
use crate::search_web;
use crate::triage_doc;

#[derive(serde::Deserialize)]
struct CallToolParams {
    name: String,
    arguments: Value,
}

pub async fn handle_request(
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
                "tools": [
                    man_page::tool_definition(),
                    fetch_url::tool_definition(),
                    read_doc::tool_definition(),
                    search_doc::tool_definition(),
                    search_web::tool_definition(),
                    &triage_doc::tool_definition()]
            }),
        }),
        "tools/call" => {
            let params: CallToolParams = serde_json::from_value(req.params.unwrap_or(Value::Null))
                .map_err(|_| mcp::invalid_params("bad params"))?;

            let result = match params.name.as_str() {
                "man_page" => man_page::handle_call(params.arguments).await,
                "fetch_url" => fetch_url::handle_call(params.arguments).await,
                "read_doc" => read_doc::handle_call(params.arguments),
                "search_doc" => search_doc::handle_call(params.arguments),
                "search_web" => search_web::handle_call(params.arguments).await,
                "triage_doc" => triage_doc::handle_call(params.arguments).await,
                _other => Err(mcp::invalid_params("unknown tool")),
            }?;

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

    #[tokio::test]
    async fn initialize_returns_server_info() {
        let req = make_request(Value::from(1), "initialize", None);
        let resp = handle_request(req).await.unwrap();

        assert_eq!(resp.id, 1);
        assert_eq!(resp.result["protocolVersion"], "2025-11-25");
        assert_eq!(resp.result["serverInfo"]["name"], "toolbox");
        assert_eq!(resp.result["serverInfo"]["version"], "0.1.0");
        assert!(resp.result["capabilities"]["tools"].is_object());
    }

    #[tokio::test]
    async fn tools_list_returns_tools() {
        let req = make_request(Value::from(2), "tools/list", None);
        let resp = handle_request(req).await.unwrap();

        assert_eq!(resp.id, 2);
        let tools = &resp.result["tools"];
        assert!(tools.is_array());
        assert_eq!(tools.as_array().unwrap().len(), 6);
        assert_eq!(tools[0]["name"], "man_page");
        assert_eq!(tools[1]["name"], "fetch_url");
        assert_eq!(tools[2]["name"], "read_doc");
        assert_eq!(tools[3]["name"], "search_doc");
        assert_eq!(tools[4]["name"], "search_web");
        assert_eq!(tools[5]["name"], "triage_doc");
    }

    #[tokio::test]
    async fn tools_call_unknown_tool() {
        let req = make_request(
            Value::from(3),
            "tools/call",
            Some(serde_json::json!({
                "name": "nonexistent",
                "arguments": { "topic": "ls" }
            })),
        );
        let err = handle_request(req).await.unwrap_err();

        assert!(err.id.is_none());
        assert_eq!(err.error.code, mcp::INVALID_PARAMS);
    }

    #[tokio::test]
    async fn tools_call_missing_params() {
        let req = make_request(Value::from(4), "tools/call", None);
        let err = handle_request(req).await.unwrap_err();

        assert_eq!(err.error.code, mcp::INVALID_PARAMS);
    }

    #[tokio::test]
    async fn tools_call_man_page_success() {
        let req = make_request(
            Value::from(5),
            "tools/call",
            Some(serde_json::json!({
                "name": "man_page",
                "arguments": { "topic": "ls" }
            })),
        );
        let resp = handle_request(req).await.unwrap();

        assert_eq!(resp.id, 5);
        assert_eq!(resp.result["isError"], false);
        assert!(
            !resp.result["content"][0]["text"]
                .as_str()
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn unknown_method() {
        let req = make_request(Value::from(6), "foo/bar", None);
        let err = handle_request(req).await.unwrap_err();

        assert_eq!(err.id, Some(Value::from(6)));
        assert_eq!(err.error.code, mcp::METHOD_NOT_FOUND);
    }
}
