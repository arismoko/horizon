#[cfg(any(test, feature = "test-support"))]
pub mod tests {

    /// A minimal direct-codec biome used by the world loader fixtures.
    pub const PACK_BIOME: &str = r##"{"temperature":0.8,"downfall":0.4,"has_precipitation":true,"effects":{"water_color":"#2301ef"},"carvers":[],"features":[]}"##;
}
