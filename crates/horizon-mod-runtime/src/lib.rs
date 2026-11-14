//! Engine-independent storage and registration machinery shared by the game and mod API.
pub mod attachments;
pub mod byte_channel;
/// A level's capability generations, which kept capabilities compare (#3446).
pub mod capability_generations;
pub mod dispatch_failures;
/// Load-owned registry names and numeric slots, without game or API key types.
pub mod registry_keys;

pub mod block_definitions;
pub mod block_entities;
/// Immutable hook subscription bitsets, resolved once at installation.
pub mod subscribers;
/// Session-only watered-area indexes, shared by server gameplay and the mod disable boundary.
pub mod water_tickets;
