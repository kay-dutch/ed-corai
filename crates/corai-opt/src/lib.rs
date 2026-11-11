//! Objectives, constraints and search over ship outfittings.

pub mod catalog;
pub mod metrics;
pub mod search;

pub use metrics::{Better, DamageProfile, Metric, MetricContext, MetricError};
