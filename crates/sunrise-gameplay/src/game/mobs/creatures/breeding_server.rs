use super::*;

impl crate::game::ServerGame<'_> {
    /// `Animal.finalizeSpawnChildFromBreeding` before the experience orb.
    pub(in crate::game) fn finalize_breeding(
        &mut self,
        parent: &Mob,
        partner: u32,
        child: Option<&Mob>,
        order: OrbOrder,
        reset_parent: bool,
    ) {
        if let Some(cause) = breeding_credit(&mut self.server.love_cause, parent.id, partner) {
            let parent_view = self.mob_entity(parent);
            let partner_view = self
                .common
                .mobs
                .iter()
                .find(|m| m.id == partner)
                .map(|m| self.mob_entity(m))
                .unwrap_or_else(|| parent_view.clone());
            let child_view = child.map(|child| self.mob_entity(child));
            let who = cause.map_or(
                super::super::commands::Target::Player,
                super::super::commands::Target::Guest,
            );
            self.for_player(who, |g| {
                g.award_custom_stat("animals_bred", 1);
                g.trigger_bred(&parent_view, &partner_view, child_view.as_ref());
            });
        }
        if reset_parent && let Some(p) = self.common.mobs.iter_mut().find(|m| m.id == parent.id) {
            sunrise_sim::mobs::finalize::set_age(p, 6000, None);
            p.extra.in_love = 0;
        }
        if let Some(q) = self.common.mobs.iter_mut().find(|m| m.id == partner) {
            sunrise_sim::mobs::finalize::set_age(q, 6000, None);
            q.extra.in_love = 0;
        }
        // TurtleBreedGoal overrides Animal's finalization and never broadcasts hearts.
        if order == OrbOrder::BeforeChild && parent.species() != Species::Turtle {
            self.mob_event_particles(parent.id, 18);
        }
    }
}
