use std::io::{BufRead, BufReader, Read, Write};
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
    assert_eq!(tools.as_array().unwrap().len(), 5);
    assert_eq!(tools[0]["name"], "man_page");
    assert_eq!(tools[1]["name"], "fetch_url");
    assert_eq!(tools[2]["name"], "read_doc");
    assert_eq!(tools[3]["name"], "search_doc");
    assert_eq!(tools[4]["name"], "search_web");
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

// Requires a SearXNG instance at 172.17.0.1:8888 (or the SEARXNG_URL env var).
// Asserts structure, not specific result text, which varies over time.
#[test]
fn tools_call_search_web_success() {
    let mut h = Harness::new();

    h.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#);
    h.send_no_response(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);

    let resp = h.send(
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search_web","arguments":{"query":"rust programming language","num_results":3}}}"#,
    );

    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 2);
    assert_eq!(resp["result"]["isError"], false);
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    assert!(!text.is_empty());
    assert!(text.starts_with("1. "), "first result should be numbered");
    assert!(text.contains("http"), "results should include URLs");
    // num_results: 3 was requested; entries are joined by a blank line.
    assert!(
        text.matches("\n\n").count() < 3,
        "requested at most 3 results, got more"
    );
}

/// Serves `body` as a single HTTP response on a random local port.
/// The server handles exactly one request and then exits. It runs on a
/// background OS thread (not tokio) so the sync test code can block
/// freely without starving a single-threaded runtime.
fn start_fake_server(body: String) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("failed to bind fake server");
    let url = format!("http://{}", listener.local_addr().unwrap());

    std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut buf = [0u8; 1024];
        let _ = socket.read(&mut buf);
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = socket.write_all(response.as_bytes());
    });

    url
}

#[test]
fn doc_flow_fake_server() {
    let doc = [
        "<!doctype html>",
        "<html>",
        "<head><title>Test Page</title></head>",
        "<body>",
        "<p>The first paragraph mentions apple.</p>",
        "<p>Another paragraph, apple again.</p>",
        "<p>Nothing interesting here.</p>",
        "</body>",
        "</html>",
    ]
    .join("\n");

    let url = start_fake_server(doc.clone());
    let mut h = Harness::new();

    h.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#);
    h.send_no_response(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);

    // 1. Fetch and store the document (small -> inline branch).
    let fetch = h.send(&format!(
        r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"fetch_url","arguments":{{"url":"{url}"}}}}}}"#
    ));
    assert_eq!(fetch["result"]["isError"], false);
    let text = fetch["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains(&format!("({} bytes, text/html)", doc.len())));
    let id = text
        .strip_prefix("[stored: ")
        .and_then(|s| s.split(' ').next())
        .unwrap();
    assert_eq!(id.len(), 64);

    // 2. Read the whole document back (smaller than the default limit).
    let read = h.send(&format!(
        r#"{{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{{"name":"read_doc","arguments":{{"id":"{id}"}}}}}}"#
    ));
    assert_eq!(read["result"]["isError"], false);
    let read_text = read["result"]["content"][0]["text"].as_str().unwrap();
    assert!(read_text.contains("Test Page"));
    assert!(!read_text.contains("[more:"), "small doc should not be truncated");

    // 3. Search: "apple" is on lines 5 and 6 (1-based); byte offsets are line starts.
    let line5 = doc.find("<p>The first").unwrap();
    let line6 = doc.find("<p>Another").unwrap();
    let search = h.send(&format!(
        r#"{{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{{"name":"search_doc","arguments":{{"id":"{id}","pattern":"apple"}}}}}}"#
    ));
    assert_eq!(search["result"]["isError"], false);
    let search_text = search["result"]["content"][0]["text"].as_str().unwrap();
    assert!(search_text.contains("2 match(es)"));
    assert!(search_text.contains(&format!("line 5, byte {line5}:")));
    assert!(search_text.contains(&format!("line 6, byte {line6}:")));

    // 4. Cross-tool: reading from the reported offset shows line 6, not line 5.
    let read2 = h.send(&format!(
        r#"{{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{{"name":"read_doc","arguments":{{"id":"{id}","offset":{line6}}}}}}}"#
    ));
    let read2_text = read2["result"]["content"][0]["text"].as_str().unwrap();
    assert!(read2_text.contains("apple again"));
    assert!(!read2_text.contains("first paragraph"));

    // 5. Zero matches is a normal result, not an error.
    let none = h.send(&format!(
        r#"{{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{{"name":"search_doc","arguments":{{"id":"{id}","pattern":"banana"}}}}}}"#
    ));
    assert_eq!(none["result"]["isError"], false);
    assert!(none["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("0 match(es)"));

    // 6. Invalid id is a tool error (isError), not a protocol error.
    let bad = h.send(
        r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"read_doc","arguments":{"id":"zz"}}}"#,
    );
    assert_eq!(bad["result"]["isError"], true);
    assert!(bad["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("invalid document id"));
}

#[test]
fn large_doc_preview_and_paging() {
    let mut doc = String::from("<html>\n");
    for i in 0..700 {
        let extra = if i < 25 { " needleword" } else { "" };
        doc.push_str(&format!(
            "<p>line {i:03}{extra} filler filler filler filler filler filler</p>\n"
        ));
    }
    doc.push_str("</html>\n");
    assert!(doc.len() > 32 * 1024, "doc must exceed the inline threshold");

    let url = start_fake_server(doc.clone());
    let mut h = Harness::new();

    h.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#);
    h.send_no_response(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);

    // 1. Large doc -> preview branch, not the full body.
    let fetch = h.send(&format!(
        r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"fetch_url","arguments":{{"url":"{url}"}}}}}}"#
    ));
    assert_eq!(fetch["result"]["isError"], false);
    let text = fetch["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.starts_with("Document stored: "));
    assert!(text.contains("Preview (first 2048 bytes):"));
    assert!(text.len() < 4096, "full body must not be inlined");
    // The id is the only 64-char whitespace-separated token in the response.
    let id = text.split_whitespace().find(|w| w.len() == 64).unwrap();

    // 2. Default read: first 4096 bytes plus a [more] pointer.
    let read = h.send(&format!(
        r#"{{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{{"name":"read_doc","arguments":{{"id":"{id}"}}}}}}"#
    ));
    let read_text = read["result"]["content"][0]["text"].as_str().unwrap();
    assert!(read_text.contains(&format!("[more: bytes 4096..{} available]", doc.len())));

    // 3. Explicit limit, then continue at the offset.
    let read100 = h.send(&format!(
        r#"{{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{{"name":"read_doc","arguments":{{"id":"{id}","limit":100}}}}}}"#
    ));
    let read100_text = read100["result"]["content"][0]["text"].as_str().unwrap();
    assert!(read100_text.contains("bytes 0..100:"));
    assert!(read100_text.contains(&format!("[more: bytes 100..{} available]", doc.len())));

    let read100_200 = h.send(&format!(
        r#"{{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{{"name":"read_doc","arguments":{{"id":"{id}","offset":100,"limit":100}}}}}}"#
    ));
    assert!(read100_200["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("bytes 100..200:"));

    // 4. Search hits the 20-match cap: 25 matches, 5 hidden.
    let first_line_start = doc.find("<p>line 000").unwrap();
    let search = h.send(&format!(
        r#"{{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{{"name":"search_doc","arguments":{{"id":"{id}","pattern":"needleword"}}}}}}"#
    ));
    let search_text = search["result"]["content"][0]["text"].as_str().unwrap();
    assert!(search_text.contains("25 match(es)"));
    assert!(search_text.contains(&format!("line 2, byte {first_line_start}:")));
    assert!(search_text.contains("... and 5 more matches"));
}
