//! # RMCP Spike — Validate rmcp crate (v1.7.0) for MCP server with stdio transport
//!
//! **THROWAWAY SPIKE** — not production code. Used to validate the API patterns
//! for Phase 1 (T22) reference.
//!
//! ## What this proves
//! 1. rmcp `#[tool_router(server_handler)]` macro works for defining tools
//! 2. `ServiceExt::serve(stdio())` starts a server on stdin/stdout
//! 3. `service.waiting().await` keeps the server alive
//! 4. Tool returns reach the client as `CallToolResult`
//!
//! ## How to test
//! ```sh
//! # Build the example
//! cargo build -p kaelo-mcp --example rmcp_spike
//!
//! # Test with MCP Inspector (interactive)
//! npx @modelcontextprotocol/inspector cargo run -p kaelo-mcp --example rmcp_spike
//!
//! # Or pipe JSON-RPC manually:
//! echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"0.1"}}}' \
//!   | cargo run -p kaelo-mcp --example rmcp_spike
//! ```
//!
//! ## API Pattern Summary (for Phase 1 reference)
//!
//! ### 1. Required Cargo.toml features
//! ```toml
//! rmcp = { version = "=1.7.0", features = ["server", "macros", "transport-io"] }
//! schemars = "0.8"  # Needed by rmcp macros for JsonSchema derives
//! ```
//!
//! ### 2. Define a tool handler struct
//! ```rust
//! #[derive(Debug, Clone)]
//! struct MyServer;
//! ```
//!
//! ### 3. Use `#[tool_router(server_handler)]` on impl block
//! - `server_handler` flag auto-generates `impl ServerHandler for MyServer`
//! - Each `#[tool]` method becomes an MCP tool
//! - Tool methods receive `Parameters<T>` for typed params, or no params
//! - Return types: `String`, `Json<T>`, or `Result<CallToolResult, McpError>`
//!
//! ### 4. Start server
//! ```rust
//! use rmcp::{ServiceExt, transport::stdio};
//! let service = MyServer.serve(stdio()).await?;
//! service.waiting().await?;
//! ```
//!
//! ### 5. Key imports
//! ```rust
//! use rmcp::{
//!     ServerHandler,           // Trait for server handlers
//!     ServiceExt,              // .serve() method
//!     tool, tool_handler,      // Attribute macros (tool_handler auto via server_handler flag)
//!     tool_router,             // Attribute macro for tool routing
//!     transport::stdio,        // stdio() transport constructor
//!     handler::server::wrapper::{Json, Parameters},  // Parameter/response wrappers
//! };
//! ```

use anyhow::Result;
use rmcp::{tool, tool_router, transport::stdio, ServiceExt};
use tracing_subscriber::EnvFilter;

/// The MCP server handler. In production, this would hold state (route cache,
/// fetch backends, etc.). For the spike, it's stateless.
#[derive(Debug, Clone)]
struct KaeloSpike;

/// Tool definitions via `#[tool_router]` macro.
///
/// The `server_handler` flag auto-emits:
/// ```rust
/// #[tool_handler]
/// impl ServerHandler for KaeloSpike {}
/// ```
/// so we don't need to write the `impl ServerHandler` block manually.
///
/// ## Tool method signatures
///
/// No params, simple string return:
/// ```rust
/// #[tool(description = "...")]
/// fn my_tool(&self) -> String { "hello".into() }
/// ```
///
/// Typed params (requires Deserialize + JsonSchema):
/// ```rust
/// #[derive(serde::Deserialize, schemars::JsonSchema)]
/// struct MyParams { url: String }
///
/// #[tool(description = "...")]
/// fn my_tool(&self, Parameters(params): Parameters<MyParams>) -> String { ... }
/// ```
///
/// Structured output:
/// ```rust
/// #[derive(serde::Serialize, schemars::JsonSchema)]
/// struct MyOutput { result: String }
///
/// #[tool(description = "...")]
/// fn my_tool(&self) -> Json<MyOutput> { Json(MyOutput { result: "ok".into() }) }
/// ```
///
/// Full control:
/// ```rust
/// #[tool(description = "...")]
/// async fn my_tool(&self, ctx: RequestContext<RoleServer>) -> Result<CallToolResult, McpError> { ... }
/// ```
#[tool_router(server_handler)]
impl KaeloSpike {
    /// Simple tool: no parameters, returns a plain string.
    /// The rmcp framework wraps String returns into CallToolResult automatically.
    #[tool(name = "hello", description = "Greet from Kaelo MCP server")]
    fn hello(&self) -> String {
        "Hello from Kaelo".to_string()
    }
}

// Note: No need for `impl ServerHandler for KaeloSpike {}` here —
// `server_handler` flag in `#[tool_router]` generates it automatically.
// The generated impl provides:
//   - `get_info()` → advertises tools capability
//   - `list_tools()` → returns metadata for all #[tool] methods
//   - `call_tool()` → dispatches to the right method by name

#[tokio::main]
async fn main() -> Result<()> {
    // Log to stderr so stdout is clean for MCP JSON-RPC traffic
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive(tracing::Level::DEBUG.into()))
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    tracing::info!("Starting Kaelo MCP spike server (rmcp v1.7.0)");

    let service = KaeloSpike.serve(stdio()).await.inspect_err(|e| {
        tracing::error!("Serving error: {:?}", e);
    })?;

    tracing::info!("Server connected, waiting for client messages...");
    service.waiting().await?;

    tracing::info!("Server shut down cleanly");
    Ok(())
}
