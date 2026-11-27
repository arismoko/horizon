use super::*;

/// The hook points. Each does nothing in Sunrise.
pub trait GameHooks<'__sg>: crate::game::SideGame<'__sg> {
    /// Whether the fishing bobber `bobber` at `position` in `level`, flying or floating outside
    /// water and not hooked to an entity, is in a medium a mod lets it fish in
    /// (`fishing_medium`): lava, the void below an island, a cloud. `owner` is the player who
    /// cast it and `owner_position` where they stand. The first mod to answer wins. Asked each
    /// tick on the server, with the caster's state swapped in, and on each client that draws
    /// the bobber; only called when a mod listens.
    fn native_fishing_medium(
        &mut self,
        level: LevelRef,
        bobber: EntityRef,
        position: [f64; 3],
        owner: Option<PlayerRef>,
        owner_position: Option<[f64; 3]>,
        medium: &mut Option<Answered<mod_api::FishingMedium>>,
    ) {
        crate::game::native_hooks::fire::FireHooks::fire_fishing_medium(
            self,
            level,
            bobber,
            position,
            owner,
            owner_position,
            medium,
        );
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> GameHooks<'__sg> for G {}
