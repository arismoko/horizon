//! Owned registration data, independent of package parsing and engine storage.

/// A declared block entity type and the block identifiers it supports.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct BlockEntityTypeDecl {
    /// Namespaced type identifier validated during registration.
    pub id: String,
    /// Supported block identifiers, in declaration order.
    pub blocks: Vec<String>,
}
