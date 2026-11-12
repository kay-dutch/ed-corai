//! ED Corai's MCP server: the corai-service tools over the Model Context Protocol.
//!
//! Tool definitions and the server instructions are kept in `tools.json` (identical to the Node
//! server's). Tools run on blocking worker threads so a long optimisation never stalls the
//! connection. When a newer release exists, the next tool result carries a one-time notice.

pub mod update;

use std::sync::{Arc, LazyLock, Mutex};

use corai_service::dispatch::{CallError, call};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    InitializeResult, ListToolsResult, PaginatedRequestParams, ServerCapabilities, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler, ServiceExt};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct Definitions {
    instructions: String,
    tools: Vec<Tool>,
}

static DEFINITIONS: LazyLock<Definitions> = LazyLock::new(|| {
    serde_json::from_str(include_str!("tools.json"))
        .expect("tools.json holds valid MCP tool definitions")
});

/// `JSON.stringify(x, null, 1)`: one-space indentation, as the Node server answers.
fn pretty(v: &Value) -> String {
    let mut out = Vec::new();
    let mut ser = serde_json::Serializer::with_formatter(
        &mut out,
        serde_json::ser::PrettyFormatter::with_indent(b" "),
    );
    if serde::Serialize::serialize(v, &mut ser).is_err() {
        return v.to_string();
    }
    String::from_utf8(out).unwrap_or_else(|_| v.to_string())
}

/// The MCP server state: the pending update notice (shown once).
#[derive(Debug, Clone, Default)]
pub struct CoraiServer {
    notice: Arc<Mutex<Option<String>>>,
}

impl CoraiServer {
    /// A server that checks for a newer release in the background unless `check_updates` is false.
    pub fn new(check_updates: bool) -> Self {
        let server = Self::default();
        if check_updates {
            let notice = Arc::clone(&server.notice);
            std::thread::spawn(move || {
                let status = update::check_for_updates();
                if status["updateAvailable"] == true
                    && let Ok(mut n) = notice.lock()
                {
                    *n = Some(format!(
                        "{} Offer the update to the user; install_update installs it only after they agree.",
                        status["message"].as_str().unwrap_or_default()
                    ));
                }
            });
        }
        server
    }

    fn take_notice(&self) -> Option<String> {
        self.notice.lock().ok().and_then(|mut n| n.take())
    }
}

/// Runs a tool by name; update tools need network access and live here, the rest in the service.
fn run_tool(name: &str, args: &Value) -> Result<Value, CallError> {
    match name {
        "check_for_updates" => Ok(update::check_for_updates()),
        "install_update" => Err(CallError::Arguments {
            tool: name.into(),
            message: "Automatic installation is not available in this build yet. Download the latest release from GitHub and replace the program.".into(),
        }),
        _ => call(name, args),
    }
}

impl ServerHandler for CoraiServer {
    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("ed-corai", env!("CARGO_PKG_VERSION")))
            .with_instructions(DEFINITIONS.instructions.clone())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult::with_all_items(DEFINITIONS.tools.clone()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let name = request.name.to_string();
        if !DEFINITIONS.tools.iter().any(|t| t.name == name) {
            return Err(McpError::invalid_params(
                format!("Tool {name} not found"),
                None,
            ));
        }
        let args = request
            .arguments
            .map_or(Value::Object(Default::default()), Value::Object);
        let task = name.clone();
        let outcome = tokio::task::spawn_blocking(move || run_tool(&task, &args))
            .await
            .map_err(|e| McpError::internal_error(format!("tool {name} failed: {e}"), None))?;
        let result = match outcome {
            Ok(v) => {
                let mut content = vec![ContentBlock::text(pretty(&v))];
                if let Some(n) = self.take_notice() {
                    content.push(ContentBlock::text(format!("Update notice: {n}")));
                }
                CallToolResult::success(content)
            }
            Err(CallError::Arguments { message, tool }) => {
                return Err(McpError::invalid_params(
                    format!("Invalid arguments for tool {tool}: {message}"),
                    None,
                ));
            }
            Err(e) => CallToolResult::error(vec![ContentBlock::text(e.to_string())]),
        };
        Ok(result.into())
    }
}

/// Serves MCP over stdin/stdout until the client disconnects.
///
/// # Errors
/// The transport fails to start or ends with an error.
pub async fn serve_stdio(
    check_updates: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let service = CoraiServer::new(check_updates)
        .serve(rmcp::transport::stdio())
        .await?;
    service.waiting().await?;
    Ok(())
}
