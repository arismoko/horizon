mod dimension;
pub use dimension::DimensionSettings;
mod __private {
    /// The levels the mods registered, for the executable to hand to the
    /// engine before any world opens.
    pub use crate::dimension::installed_dimensions;
}
