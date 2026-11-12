//! Tool calls by name with JSON arguments, as an MCP server or the command line receives them.

use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::error::ServiceError;
use crate::{info, ops};

/// Why a tool call failed: bad arguments, an unknown tool, or the tool's own error.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum CallError {
    #[error("Invalid arguments for tool {tool}: {message}")]
    Arguments { tool: String, message: String },
    #[error("Unknown tool {0}")]
    UnknownTool(String),
    #[error(transparent)]
    Service(#[from] ServiceError),
}

fn args<T: DeserializeOwned>(tool: &str, v: &Value) -> Result<T, CallError> {
    let v = if v.is_null() {
        Value::Object(Default::default())
    } else {
        v.clone()
    };
    serde_json::from_value(v).map_err(|e| CallError::Arguments {
        tool: tool.into(),
        message: e.to_string(),
    })
}

fn opt_str<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

/// Runs a read-only or build tool by name. Update tools live with the transport (they need I/O).
///
/// # Errors
/// Unknown tool, invalid arguments, or the tool's error.
pub fn call(tool: &str, a: &Value) -> Result<Value, CallError> {
    Ok(match tool {
        "list_ships" => info::list_ships()?,
        "ship_details" => {
            let ctx: Option<crate::context::ContextSpec> =
                a.get("context").map(|c| args(tool, c)).transpose()?;
            info::ship_details(opt_str(a, "ship"), opt_str(a, "build"), ctx.as_ref())?
        }
        "list_module_groups" => info::list_module_groups(opt_str(a, "category")),
        "module_info" => {
            let group = opt_str(a, "group").ok_or_else(|| CallError::Arguments {
                tool: tool.into(),
                message: "group is required".into(),
            })?;
            info::module_info(group, a.get("class").and_then(Value::as_u64))?
        }
        "list_metrics" => info::list_metrics(),
        "evaluate_build" => ops::evaluate_build(&args(tool, a)?)?,
        "export_build" => ops::export_build(&args(tool, a)?)?,
        "optimize_build" => ops::optimize_build(&args(tool, a)?)?,
        "compare_ships" => ops::compare_ships(&args(tool, a)?)?,
        other => return Err(CallError::UnknownTool(other.into())),
    })
}
