//! Integration tests for the Kaelo MCP server.
//!
//! Non-network tests create a KaeloServer with in-memory storage.
//! Subprocess JSON-RPC tests (behind `#[ignore]`) spawn the `kaelo` binary.
//! Run all: `cargo test -p kaelo-mcp -- --include-ignored`

use std::io::{BufRead, BufReader, Read as _, Write as _};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::Duration;

use kaelo_core::storage::Storage;
use kaelo_mcp::KaeloServer;

fn find_kaelo_binary() -> String {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR set");
    let workspace_root = std::path::Path::new(&manifest_dir)
        .parent()
        .expect("parent dir");
    let debug_path = workspace_root.join("target").join("debug").join("kaelo");
    if debug_path.exists() {
        return debug_path.to_string_lossy().to_string();
    }
    // Fallback: assume it's in PATH
    "kaelo".to_string()
}

struct McpProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl McpProcess {
    fn spawn() -> Self {
        let binary = find_kaelo_binary();
        let mut child = Command::new(&binary)
            .arg("serve")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap_or_else(|e| panic!("failed to spawn {binary}: {e}"));

        let stdin = child.stdin.take().expect("stdin should be piped");
        let stdout = BufReader::new(child.stdout.take().expect("stdout should be piped"));

        Self {
            child,
            stdin,
            stdout,
        }
    }

    fn send(&mut self, msg: &serde_json::Value) {
        let json = serde_json::to_string(msg).expect("serialize JSON");
        let framed = format!("Content-Length: {}\r\n\r\n{}", json.len(), json);
        self.stdin
            .write_all(framed.as_bytes())
            .expect("write to stdin");
        self.stdin.flush().expect("flush stdin");
    }

    fn recv(&mut self) -> serde_json::Value {
        // Read Content-Length header
        let mut header_line = String::new();
        self.stdout
            .read_line(&mut header_line)
            .expect("read header line");

        let len: usize = header_line
            .strip_prefix("Content-Length: ")
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or_else(|| panic!("malformed header: {header_line:?}"));

        // Read empty line separator
        let mut sep = String::new();
        self.stdout.read_line(&mut sep).expect("read separator");

        // Read JSON body
        let mut buf = vec![0u8; len];
        self.stdout.read_exact(&mut buf).expect("read JSON body");
        let body = String::from_utf8(buf).expect("body is UTF-8");
        serde_json::from_str(&body).unwrap_or_else(|e| panic!("parse JSON: {e}\nbody: {body}"))
    }
}

impl Drop for McpProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn server_default_creates_in_memory_storage() {
    let _server = KaeloServer::default();
}

#[test]
fn server_new_with_in_memory_storage() {
    let storage = Storage::open(":memory:").expect("in-memory storage should open");
    let _server = KaeloServer::new(storage);
}

#[test]
#[ignore = "requires built kaelo binary"]
fn mcp_initialize_returns_protocol_version() {
    let mut proc = McpProcess::spawn();

    proc.send(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "test", "version": "0.1.0" }
        }
    }));

    let resp = proc.recv();
    assert_eq!(resp["id"], 1, "response id should match request");
    let result = resp
        .get("result")
        .expect("initialize should return result (not error)");
    assert!(
        result.get("protocolVersion").is_some(),
        "result should contain protocolVersion"
    );
}

#[test]
#[ignore = "requires built kaelo binary"]
fn mcp_tools_list_returns_all_six_tools() {
    let mut proc = McpProcess::spawn();

    // Initialize first
    proc.send(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "test", "version": "0.1.0" }
        }
    }));
    let _init_resp = proc.recv();

    // Send initialized notification
    proc.send(&serde_json::json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    }));

    // List tools
    proc.send(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list",
        "params": {}
    }));

    let resp = proc.recv();
    assert_eq!(resp["id"], 2);
    let tools = resp["result"]["tools"]
        .as_array()
        .expect("tools should be an array");

    let tool_names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();

    for expected in &[
        "web_fetch",
        "web_search",
        "cache_status",
        "cache_clear",
        "ping",
        "fetch_urls",
    ] {
        assert!(
            tool_names.iter().any(|n| n == expected),
            "tools/list should contain '{expected}', got: {tool_names:?}"
        );
    }
}

#[test]
#[ignore = "requires built kaelo binary"]
fn mcp_ping_returns_pong() {
    let mut proc = McpProcess::spawn();

    proc.send(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "test", "version": "0.1.0" }
        }
    }));
    let _init_resp = proc.recv();

    proc.send(&serde_json::json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    }));

    proc.send(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "ping",
            "arguments": {}
        }
    }));

    let resp = proc.recv();
    assert_eq!(resp["id"], 2);
    let content = resp["result"]["content"]
        .as_array()
        .expect("result should have content array");
    let text = content[0]["text"]
        .as_str()
        .expect("content[0] should have text");
    assert!(
        text.contains("pong"),
        "ping tool should return 'pong', got: {text}"
    );
}

#[test]
#[ignore = "requires built kaelo binary"]
fn mcp_cache_status_returns_structure() {
    let mut proc = McpProcess::spawn();

    proc.send(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "test", "version": "0.1.0" }
        }
    }));
    let _init_resp = proc.recv();

    proc.send(&serde_json::json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    }));

    proc.send(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "cache_status",
            "arguments": {}
        }
    }));

    let resp = proc.recv();
    assert_eq!(resp["id"], 2);
    let content = resp["result"]["content"]
        .as_array()
        .expect("result should have content array");
    let text = content[0]["text"]
        .as_str()
        .expect("content[0] should have text");
    assert!(
        text.contains("Route cache"),
        "cache_status should mention 'Route cache', got: {text}"
    );
    assert!(
        text.contains("Content cache"),
        "cache_status should mention 'Content cache', got: {text}"
    );
}

#[test]
#[ignore = "requires built kaelo binary"]
fn mcp_cache_clear_returns_success() {
    let mut proc = McpProcess::spawn();

    proc.send(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "test", "version": "0.1.0" }
        }
    }));
    let _init_resp = proc.recv();

    proc.send(&serde_json::json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    }));

    proc.send(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "cache_clear",
            "arguments": {}
        }
    }));

    let resp = proc.recv();
    assert_eq!(resp["id"], 2);
    let content = resp["result"]["content"]
        .as_array()
        .expect("result should have content array");
    let text = content[0]["text"]
        .as_str()
        .expect("content[0] should have text");
    assert!(
        text.contains("Cache cleared"),
        "cache_clear should confirm clearing, got: {text}"
    );
}

#[test]
#[ignore = "requires network"]
fn mcp_web_fetch_returns_markdown() {
    let mut proc = McpProcess::spawn();

    proc.send(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "test", "version": "0.1.0" }
        }
    }));
    let _init_resp = proc.recv();

    proc.send(&serde_json::json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    }));

    proc.send(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "web_fetch",
            "arguments": { "url": "https://example.com" }
        }
    }));

    // Give it time to fetch
    std::thread::sleep(Duration::from_secs(10));

    let resp = proc.recv();
    assert_eq!(resp["id"], 2);
    assert!(
        resp.get("result").is_some(),
        "web_fetch should return result, got: {resp}"
    );
    let content = resp["result"]["content"]
        .as_array()
        .expect("result should have content array");
    let text = content[0]["text"]
        .as_str()
        .expect("content[0] should have text");
    assert!(
        !text.is_empty(),
        "web_fetch should return non-empty content"
    );
}

#[test]
#[ignore = "requires built kaelo binary"]
fn mcp_fetch_urls_empty_list_returns_error() {
    let mut proc = McpProcess::spawn();

    proc.send(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "test", "version": "0.1.0" }
        }
    }));
    let _init_resp = proc.recv();

    proc.send(&serde_json::json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    }));

    proc.send(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "fetch_urls",
            "arguments": { "urls": [] }
        }
    }));

    let resp = proc.recv();
    assert_eq!(resp["id"], 2);
    let error = resp
        .get("result")
        .and_then(|r| r.get("isError"))
        .or_else(|| {
            resp.get("error")
                .is_some()
                .then_some(&serde_json::Value::Bool(true))
        })
        .expect("empty URL list should return error");
    assert!(
        error.as_bool().unwrap_or(false) || resp.get("error").is_some(),
        "empty URL list should produce an error response, got: {resp}"
    );
}

#[test]
#[ignore = "requires built kaelo binary"]
fn mcp_fetch_urls_too_many_returns_error() {
    let mut proc = McpProcess::spawn();

    proc.send(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "test", "version": "0.1.0" }
        }
    }));
    let _init_resp = proc.recv();

    proc.send(&serde_json::json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    }));

    let urls: Vec<String> = (0..11).map(|i| format!("https://example{i}.com")).collect();

    proc.send(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "fetch_urls",
            "arguments": { "urls": urls }
        }
    }));

    let resp = proc.recv();
    assert_eq!(resp["id"], 2);
    let error = resp
        .get("result")
        .and_then(|r| r.get("isError"))
        .or_else(|| {
            resp.get("error")
                .is_some()
                .then_some(&serde_json::Value::Bool(true))
        })
        .expect(">10 URLs should return error");
    assert!(
        error.as_bool().unwrap_or(false) || resp.get("error").is_some(),
        ">10 URLs should produce an error response, got: {resp}"
    );
}
