//! A sample crate with intentional code quality issues for descendit stress testing.
//!
//! This crate simulates a small data pipeline with various structural problems:
//! - Bloated functions that do too much
//! - Duplicated processing logic
//! - Types with high state cardinality (bool soup)
//! - Poor code economy (many private helpers, few public functions)

pub mod config;
pub mod ingest;
pub mod transform;
pub mod output;
