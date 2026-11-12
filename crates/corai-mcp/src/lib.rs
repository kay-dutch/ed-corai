//! ED Corai's MCP server: the corai-service tools over the Model Context Protocol.
//!
//! Tool definitions and the server instructions are kept in `tools.json` (identical to the Node
//! server's). Tools run on blocking worker threads so a long optimisation never stalls the
//! connection. When a newer release exists, the next tool result carries a one-time notice.

mod activity;
mod journal_tools;
pub use corai_update as update;

use std::sync::{Arc, LazyLock, Mutex};
use std::time::Instant;

use corai_activity::{Message, Reporter};

use corai_service::dispatch::{CallError, call};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    InitializeResult, ListToolsResult, PaginatedRequestParams, ServerCapabilities, Tool,
};
use rmcp::service::{NotificationContext, RequestContext};
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

/// The MCP server state: the pending update notice (shown once) and the reports to the window.
#[derive(Debug, Clone, Default)]
pub struct CoraiServer {
    notice: Arc<Mutex<Option<String>>>,
    reporter: Arc<Reporter>,
    /// The saved defaults last mentioned to the AI app, so each set is mentioned once.
    told_defaults: Arc<Mutex<Vec<String>>>,
}

impl CoraiServer {
    /// A server that checks for a newer release in the background unless `check_updates` is false.
    pub fn new(check_updates: bool) -> Self {
        let server = Self::default();
        server
            .reporter
            .keep_announcing(std::time::Duration::from_secs(3));
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

    /// Fills in the player's saved defaults (set in the window) where the call leaves them out.
    /// Returns a note for the AI app the first time a given set is used.
    fn apply_defaults(&self, tool: &str, args: &mut Value) -> Option<String> {
        let path = corai_activity::defaults::defaults_path()?;
        let defaults = corai_activity::defaults::Defaults::load(&path);
        let takes = |key: &str| {
            DEFINITIONS
                .tools
                .iter()
                .find(|t| t.name == tool)
                .and_then(|t| t.input_schema.get("properties"))
                .is_some_and(|p| p.get(key).is_some())
        };
        let applied = defaults.apply(args, takes("context"), takes("engineering"));
        let mut told = self.told_defaults.lock().ok()?;
        if applied.is_empty() || *told == applied {
            return None;
        }
        let note = format!(
            "The player's saved defaults (ED Corai window) filled in what this call left out: {}. \
             Mention it when it affects the answer; pass a value explicitly to override.",
            applied.join(", ")
        );
        *told = applied;
        Some(note)
    }
}

/// Why a tool call failed: bad arguments or a failed service call, or a failed adapter tool.
enum ToolFailure {
    Call(CallError),
    Failed(String),
}

impl std::fmt::Display for ToolFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Call(e) => e.fmt(f),
            Self::Failed(e) => f.write_str(e),
        }
    }
}

impl From<CallError> for ToolFailure {
    fn from(e: CallError) -> Self {
        Self::Call(e)
    }
}

/// Runs a tool by name; update and journal tools need network or file access and live here, the
/// rest in the service.
fn run_tool(name: &str, args: &Value) -> Result<Value, ToolFailure> {
    match name {
        "check_for_updates" => Ok(update::check_for_updates()),
        "install_update" => {
            if args["confirm"] != true {
                return Err(CallError::Arguments {
                    tool: name.into(),
                    message: "confirm must be true: install only after the user agreed.".into(),
                }
                .into());
            }
            update::install_update(args["version"].as_str()).map_err(ToolFailure::Failed)
        }
        "current_ship" => {
            journal_tools::current_ship(corai_journal::journal_dir().as_deref(), args)
                .map_err(ToolFailure::Failed)
        }
        "recent_ships" => {
            journal_tools::recent_ships(corai_journal::journal_dir().as_deref(), args)
                .map_err(ToolFailure::Failed)
        }
        _ => Ok(call(name, args)?),
    }
}

/// Runs any tool by name, as the MCP server would (for the command line).
///
/// # Errors
/// The tool's error message.
pub fn run_tool_text(name: &str, args: &Value) -> Result<Value, String> {
    if !DEFINITIONS.tools.iter().any(|t| t.name == name) {
        return Err(format!("Unknown tool {name}"));
    }
    run_tool(name, args).map_err(|e| e.to_string())
}

impl ServerHandler for CoraiServer {
    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("ed-corai", env!("CARGO_PKG_VERSION")))
            .with_instructions(DEFINITIONS.instructions.clone())
    }

    async fn on_initialized(&self, context: NotificationContext<RoleServer>) {
        if let Some(peer) = context.peer.peer_info() {
            let client = &peer.client_info;
            let name = client.title.clone().unwrap_or_else(|| client.name.clone());
            let reporter = Arc::clone(&self.reporter);
            // Connecting may wait briefly; keep it off the protocol task.
            std::thread::spawn(move || {
                reporter.announce();
                reporter.set_client(&name);
            });
        }
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
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let name = request.name.to_string();
        if !DEFINITIONS.tools.iter().any(|t| t.name == name) {
            return Err(McpError::invalid_params(
                format!("Tool {name} not found"),
                None,
            ));
        }
        let mut args = request
            .arguments
            .map_or(Value::Object(Default::default()), Value::Object);
        let defaults_note = self.apply_defaults(&name, &mut args);
        if let Some(peer) = context.peer.peer_info() {
            let client = &peer.client_info;
            self.reporter
                .set_client(client.title.as_deref().unwrap_or(&client.name));
        }
        let task = name.clone();
        let reporter = Arc::clone(&self.reporter);
        let outcome = tokio::task::spawn_blocking(move || {
            let id = reporter.next_id();
            reporter.send(&Message::Started {
                id,
                tool: task.clone(),
                detail: activity::describe_call(&args),
            });
            let started = Instant::now();
            let outcome = run_tool(&task, &args);
            let (detail, link) = match &outcome {
                Ok(v) => activity::describe_result(&task, v),
                Err(e) => (e.to_string(), None),
            };
            reporter.send(&Message::Finished {
                id,
                ok: outcome.is_ok(),
                millis: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                detail,
                link,
            });
            outcome
        })
        .await
        .map_err(|e| McpError::internal_error(format!("tool {name} failed: {e}"), None))?;
        let result = match outcome {
            Ok(v) => {
                let mut content = vec![ContentBlock::text(pretty(&v))];
                if let Some(n) = defaults_note {
                    content.push(ContentBlock::text(n));
                }
                if let Some(n) = self.take_notice() {
                    content.push(ContentBlock::text(format!("Update notice: {n}")));
                }
                CallToolResult::success(content)
            }
            Err(ToolFailure::Call(CallError::Arguments { message, tool })) => {
                return Err(McpError::invalid_params(
                    format!("Invalid arguments for tool {tool}: {message}"),
                    None,
                ));
            }
            Err(ToolFailure::Call(e)) => {
                CallToolResult::error(vec![ContentBlock::text(e.to_string())])
            }
            Err(ToolFailure::Failed(e)) => CallToolResult::error(vec![ContentBlock::text(e)]),
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
