impl crate::game::ClientGame<'_> {
    /// Re-query dynamic-light cells in sections touched by a server light update.
    pub fn ask_updated_light_emission(&mut self, key: (i32, i32), sections: &[usize]) {
        if sections.is_empty() || !self.client.client_pack.context.chunk.light.any_dynamic() {
            return;
        }
        let Some(chunk) = self.common.world.chunks.get(&key) else {
            return;
        };
        let cells = self
            .client
            .client_pack
            .context
            .chunk
            .light
            .dynamic_cells(chunk, |s| sections.contains(&s));
        let dimension = self.current_light_emission_dimension();
        for (cell, [x, y, z], state) in cells {
            let emission = self.ask_light_emission(dimension, x, y, z, state);
            self.common.store_light_emission(key, cell, emission);
        }
    }
}

use sunrise_gameplay_common::game::light_emission::LightEmissionGameExt1 as _;
