use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Deserialize, Serialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Value,
    pub method: String,
    pub params: Option<Value>,
}

#[derive(Deserialize, Serialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    pub params: Option<Value>,
}

#[derive(Deserialize, Serialize)]
pub enum JsonRpcMessage {
    Request(JsonRpcRequest),
    Notification(JsonRpcNotification),
}

#[derive(Deserialize, Serialize)]
pub struct JsonRpcResponse {
    pub id: Value,
    pub jsonrpc: String,
    pub result: Value,
}

#[derive(Deserialize, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    pub data: Option<Value>,
}

#[derive(Deserialize, Serialize)]
pub struct JsonRpcErrorResponse {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub error: JsonRpcError,
}

pub const PARSE_ERROR: i32 = -32700;
pub const INVALID_REQUEST: i32 = -32600;
pub const METHOD_NOT_FOUND: i32 = -32601;
pub const INVALID_PARAMS: i32 = -32602;
pub const INTERNAL_ERROR: i32 = -32603;

pub fn parse_message(message: &str) -> Result<JsonRpcMessage, JsonRpcError> {
    let value: Value = match serde_json::from_str(message) {
        Ok(v) => v,
        Err(_) => {
            return Err(JsonRpcError {
                code: PARSE_ERROR,
                message: "parsing json failed".into(),
                data: None,
            });
        }
    };

    match value.get("id") {
        Some(_) => match serde_json::from_value(value) {
            Ok(parsed) => Ok(JsonRpcMessage::Request(parsed)),
            Err(_) => Err(JsonRpcError {
                code: INVALID_REQUEST,
                message: "invalid request".into(),
                data: None,
            }),
        },
        None => match serde_json::from_value(value) {
            Ok(parsed) => Ok(JsonRpcMessage::Notification(parsed)),
            Err(_) => Err(JsonRpcError {
                code: INVALID_REQUEST,
                message: "invalid request".into(),
                data: None,
            }),
        },
    }
}
