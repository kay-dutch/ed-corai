//! Objectives, constraints and search over ship outfittings.

pub mod arrange;
pub mod catalog;
pub mod metrics;
pub mod search;
pub mod validate;

pub use metrics::{Better, DamageProfile, Metric, MetricContext, MetricError};
