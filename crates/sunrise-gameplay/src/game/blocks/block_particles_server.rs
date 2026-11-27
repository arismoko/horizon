impl crate::game::ServerGame<'_> {
    /// `BlockTransformer.TransformParticle.send`: its level event.
    pub(in crate::game) fn transform_particle(&mut self, particle: &str, pos: [i32; 3]) {
        let source = self
            .server
            .lan_acting_uuid
            .map(sunrise_world::level::LevelEventPlayer);
        match particle {
            "scrape" => self.level_event_from(source, 3005, pos, 0),
            "wax_on" => self.level_event_from(source, 3003, pos, 0),
            "wax_off" => self.level_event_from(source, 3004, pos, 0),
            _ => {}
        }
    }
}
