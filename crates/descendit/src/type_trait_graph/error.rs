use std::fmt;

use super::{ImplEdge, ItemId, TraitName, TypeName};

/// Type/trait graph construction or consistency error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeTraitGraphError {
    /// Duplicate type node.
    DuplicateType { ty: TypeName },
    /// Duplicate trait node.
    DuplicateTrait { tr: TraitName },
    /// Unknown type node.
    UnknownType { ty: TypeName },
    /// Unknown trait node.
    UnknownTrait { tr: TraitName },
    /// Duplicate implementation edge.
    DuplicateImpl { source: ItemId, target: TraitName },
    /// Unknown implementation edge.
    UnknownImpl { edge: ImplEdge },
    /// Trait cannot imply itself.
    SelfTraitImpl { tr: TraitName },
}

impl fmt::Display for TypeTraitGraphError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateType { ty } => write!(formatter, "duplicate type: {ty}"),
            Self::DuplicateTrait { tr } => write!(formatter, "duplicate trait: {tr}"),
            Self::UnknownType { ty } => write!(formatter, "unknown type: {ty}"),
            Self::UnknownTrait { tr } => write!(formatter, "unknown trait: {tr}"),
            Self::DuplicateImpl { source, target } => {
                write!(
                    formatter,
                    "duplicate impl edge: {source:?} -> {}",
                    target.as_str()
                )
            }
            Self::UnknownImpl { edge } => write!(formatter, "unknown impl edge: {edge}"),
            Self::SelfTraitImpl { tr } => write!(formatter, "trait implies itself: {tr}"),
        }
    }
}

impl std::error::Error for TypeTraitGraphError {}
