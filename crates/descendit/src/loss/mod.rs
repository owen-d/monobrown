//! Loss dimension implementations.
//!
//! Each loss function lives in its own submodule. The `common` module provides
//! shared helpers used across multiple dimensions.

pub(crate) mod bloat;
pub(crate) mod code_economy;
pub(crate) mod common;
pub(crate) mod coupling_density;
pub(crate) mod duplication;
pub(crate) mod state_cardinality;
