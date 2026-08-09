use std::io::{BufRead, BufReader};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

struct Harness {
    _child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Harness {
    fn new() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_toolbox"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to spawn toolbox");

        let stdin = child.stdin.take().expect("failed to take stdin");
        let stdout = BufReader::new(child.stdout.take().expect("failed to take stdout"));

        Harness {
            _child: child,
            stdin,
            stdout,
        }
    }

    /// Send a JSON-RPC message and read the response line.
    fn send(&mut self, msg: &str) -> serde_json::Value {
        self.send_raw(msg);
        self.read_response()
    }

    /// Send a message that should not produce a response (e.g. notifications).
    fn send_no_response(&mut self, msg: &str) {
        self.send_raw(msg);
    }

    fn send_raw(&mut self, msg: &str) {
        use std::io::Write;
        self.stdin
            .write_all(format!("{}\n", msg).as_bytes())
            .expect("failed to write to stdin");
        self.stdin.flush().expect("failed to flush stdin");
    }

    fn read_response(&mut self) -> serde_json::Value {
        let mut line = String::new();
        self.stdout
            .read_line(&mut line)
            .expect("failed to read from stdout");
        serde_json::from_str(&line).expect("failed to parse response as JSON")
    }
}

// --- Tests ---

#[test]
fn initialize() {
    let mut h = Harness::new();

    let resp = h.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#);

    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 1);
    assert_eq!(resp["result"]["protocolVersion"], "2025-11-25");
    assert_eq!(resp["result"]["serverInfo"]["name"], "toolbox");
}

#[test]
fn tools_list() {
    let mut h = Harness::new();

    h.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#);
    // Send the initialized notification (no response expected).
    h.send_no_response(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);

    let resp = h.send(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);

    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 2);
    let tools = &resp["result"]["tools"];
    assert!(tools.is_array());
    assert_eq!(tools.as_array().unwrap().len(), 2);
    assert_eq!(tools[0]["name"], "man_page");
    assert_eq!(tools[1]["name"], "fetch_url");
}

#[test]
fn tools_call_man_page_success() {
    let mut h = Harness::new();

    h.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#);
    h.send_no_response(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);

    let resp = h.send(r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"man_page","arguments":{"topic":"ls"}}}"#);

    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 2);
    assert_eq!(resp["result"]["isError"], false);
    let content = &resp["result"]["content"][0];
    assert_eq!(content["type"], "text");
    assert!(!content["text"].as_str().unwrap().is_empty());
}

#[test]
fn tools_call_man_page_not_found() {
    let mut h = Harness::new();

    h.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#);
    h.send_no_response(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);

    let resp = h.send(
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"man_page","arguments":{"topic":"nonexistent_topic_xyz"}}}"#,
    );

    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 2);
    assert_eq!(resp["result"]["isError"], true);
    assert!(
        resp["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("not found")
    );
}

#[test]
fn tools_call_invalid_params() {
    let mut h = Harness::new();

    h.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#);
    h.send_no_response(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);

    // Missing "name" field.
    let resp = h.send(
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"arguments":{"topic":"ls"}}}"#,
    );

    assert_eq!(resp["jsonrpc"], "2.0");
    // Errors without a request id get id: null.
    assert!(resp["id"].is_null());
    assert_eq!(resp["error"]["code"], -32602); // INVALID_PARAMS
}

#[test]
fn invalid_json() {
    let mut h = Harness::new();

    let resp = h.send("not json at all");

    assert_eq!(resp["jsonrpc"], "2.0");
    assert!(resp["id"].is_null());
    assert_eq!(resp["error"]["code"], -32700); // PARSE_ERROR
}

#[test]
fn unknown_method() {
    let mut h = Harness::new();

    let resp = h.send(r#"{"jsonrpc":"2.0","id":1,"method":"unknown/method"}"#);

    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 1);
    assert_eq!(resp["error"]["code"], -32601); // METHOD_NOT_FOUND
}

#[test]
fn full_lifecycle() {
    let mut h = Harness::new();

    // 1. Initialize
    let init = h.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#);
    assert_eq!(init["result"]["protocolVersion"], "2025-11-25");

    // 2. Initialized notification
    h.send_no_response(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);

    // 3. List tools
    let list = h.send(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
    assert_eq!(list["result"]["tools"][0]["name"], "man_page");

    // 4. Call tool
    let call = h.send(
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"man_page","arguments":{"topic":"ls","section":"1"}}}"#,
    );
    assert_eq!(call["result"]["isError"], false);
    assert!(
        !call["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn tools_call_fetch_url_success() {
    let mut h = Harness::new();

    h.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#);
    h.send_no_response(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);

    let resp = h.send(
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"fetch_url","arguments":{"url":"http://example.com"}}}"#,
    );

    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 2);
    assert_eq!(resp["result"]["isError"], false);
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("Example Domain"));
}
