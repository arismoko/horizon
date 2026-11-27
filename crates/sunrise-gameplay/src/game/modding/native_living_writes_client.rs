#[cfg(test)]
pub(in crate::game) mod tests {
    use super::super::observer_tests::tests::{Shared, UseMod};
    use crate::game::native_entity_refs::tests::with_game;
    use mod_api::{LivingRef, ModEntry, Mods};
    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

    /// Runs `test` on a game whose acting player stands at the origin, the use
    /// button down, under a mod that fills the returned record.
    pub(in crate::game) fn with_uses(
        test: impl FnOnce(&mut crate::game::ServerGame<'_>, &Shared, LivingRef),
    ) {
        let uses = Shared::default();
        let mods: Mods = vec![ModEntry::new("uses", Box::new(UseMod(uses.clone())))].into();
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                crate::game::native_entity_refs::tests::resolve_parts(game).unwrap();
                game.player =
                    Some(sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
                crate::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
                game.server_set_acting_use(true);
                let player = game.living(game.entity(20).unwrap()).unwrap();
                test(game, &uses, player);
            });
        });
    }
}
