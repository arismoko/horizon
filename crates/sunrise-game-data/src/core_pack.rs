//! The prepared core data pack Sunrise acknowledges in `KnownPacks` so servers can omit its entry data.

/// The core pack identity is versioned by the game version the resources were built for.
pub const CORE_PACK: (&str, &str, &str) = ("minecraft", "core", sunrise_core::VANILLA_VERSION);
