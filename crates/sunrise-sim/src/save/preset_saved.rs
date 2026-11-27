impl WorldGenerator {
    /// Saved preset/editor stems fill only levels that mods and enabled packs did not define.
    pub fn merge_dimensions(
        &self,
        overrides: &mut sunrise_world::worldgen::overrides::WorldgenOverrides,
    ) {
        use sunrise_world::worldgen::overrides::DIMENSIONS;
        let Some(Tag::Compound(dimensions)) = self.data.get("dimensions") else {
            unreachable!("validated dimension map")
        };
        for (id, stem) in dimensions {
            let id = id.to_string_lossy();
            if !overrides.entries(DIMENSIONS).any(|(level, _)| level == id) {
                overrides.set(
                    DIMENSIONS,
                    &id,
                    crate::scoreboard::tag_to_json(stem).to_string(),
                );
            }
        }
    }
}
