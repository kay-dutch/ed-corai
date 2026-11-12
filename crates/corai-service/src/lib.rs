//! The use cases behind ED Corai's tools, independent of any transport (MCP, command line, window).
//! Requests and results are plain serde types whose JSON matches the Node server.

pub mod changes;
pub mod context;
pub mod dispatch;
pub mod error;
pub mod info;
pub mod labels;
pub mod names;
pub mod num;
pub mod ops;
pub mod ships;
pub mod summary;

pub use error::ServiceError;
pub use summary::{BuildSummary, SummaryOptions, summarize};
