#![cfg(test)]

#[cfg(test)]
pub(crate) mod tests {
    use crate::game::server_side::native_living_effects::tests::collect_effects;
    use mod_api::{EntityRef, LivingRef, WorldEpoch};
    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;

    /// `LivingEntity.getEffectBlendFactor` on a client: the local player's
    /// darkness fades in a 22nd a tick and nausea a 150th
    /// (`MobEffect.setBlendDuration` in `MobEffects`), an ordinary effect
    /// interpolates during its first tick, a missing one reads 0, and no other
    /// entity reads.
    #[test]
    #[ignore = "daily"]
    fn a_client_reads_the_local_players_effect_blend_factors() {
        use super::super::native_entity_refs::tests::with_client_game;
        use sunrise_level::effects::EffectInstance;
        use sunrise_protocol::connection::Pipe;
        use sunrise_sim::motion::PlayerMotion;
        let close =
            |read: Option<f32>, expected: f32| read.is_some_and(|f| (f - expected).abs() < 1e-6);
        with_client_game(|game| {
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "BlendReadTest").unwrap();
            let remote = game.client.remote.as_mut().unwrap();
            remote.logged_in = true;
            remote.entity_id = 7;
            remote.profile = 42;
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            let me = game.living(game.entity(7).unwrap()).unwrap();
            assert_eq!(
                me.effect_blend_factor(
                    game,
                    mod_platform::__private::part_tests::effect_key("minecraft:darkness"),
                    0.5
                ),
                Some(0.0)
            );

            // A blended darkness starts from nothing; two client ticks later
            // it is between 1/22 and 2/22.
            game.potions
                .effects
                .list
                .push(EffectInstance::new("darkness", 200, 0));
            game.client.darkness_blend = (0.0, 0.0);
            // The same effect reads whole on the client: `getEffect`, `getActiveEffects`, `hasEffect`.
            let darkness = mod_platform::__private::part_tests::effect_key("minecraft:darkness");
            assert!(me.potion_affected(game, darkness));
            assert_eq!(me.effect(game, darkness).map(|e| e.duration), Some(200));
            assert_eq!(collect_effects(game, me).map(|list| list.len()), Ok(1));
            assert!(!me.potion_affected(
                game,
                mod_platform::__private::part_tests::effect_key("minecraft:nausea")
            ));
            assert_eq!(
                me.effect_blend_factor(
                    game,
                    mod_platform::__private::part_tests::effect_key("darkness"),
                    1.0
                ),
                Some(0.0)
            );
            game.tick_client_effects();
            game.tick_client_effects();
            assert!(close(
                me.effect_blend_factor(
                    game,
                    mod_platform::__private::part_tests::effect_key("minecraft:darkness"),
                    0.0
                ),
                1.0 / 22.0
            ));
            assert!(close(
                me.effect_blend_factor(
                    game,
                    mod_platform::__private::part_tests::effect_key("minecraft:darkness"),
                    0.5
                ),
                1.5 / 22.0
            ));

            // Nausea fades in over 150 ticks, as the screen spin ticks it.
            game.potions
                .effects
                .list
                .push(EffectInstance::new("nausea", 600, 0));
            game.client.screen_fx.tick_spinning(Some(true));
            assert!(close(
                me.effect_blend_factor(
                    game,
                    mod_platform::__private::part_tests::effect_key("minecraft:nausea"),
                    0.5
                ),
                0.5 / 150.0
            ));

            // Speed starts at 0 and interpolates to 1 over its first tick.
            game.potions
                .effects
                .list
                .push(EffectInstance::new("speed", 100, 0));
            assert_eq!(
                me.effect_blend_factor(
                    game,
                    mod_platform::__private::part_tests::effect_key("minecraft:speed"),
                    0.0
                ),
                Some(0.0)
            );
            game.tick_client_effects();
            assert_eq!(
                me.effect_blend_factor(
                    game,
                    mod_platform::__private::part_tests::effect_key("minecraft:speed"),
                    0.5
                ),
                Some(0.5)
            );

            // Gone, darkness reads 0 though its fade had not reached 0.
            game.potions.effects.list.retain(|e| e.id != "darkness");
            assert_eq!(
                me.effect_blend_factor(
                    game,
                    mod_platform::__private::part_tests::effect_key("minecraft:darkness"),
                    0.5
                ),
                Some(0.0)
            );

            // Another entity and a stale handle read nothing.

            let epoch = me.entity().epoch();
            let other = LivingRef::new(EntityRef::new(epoch, 8));
            assert_eq!(
                other.effect_blend_factor(
                    game,
                    mod_platform::__private::part_tests::effect_key("minecraft:speed"),
                    0.5
                ),
                None
            );
            let stale = LivingRef::new(EntityRef::new(WorldEpoch::new(epoch.get() + 1), 7));
            assert_eq!(
                stale.effect_blend_factor(
                    game,
                    mod_platform::__private::part_tests::effect_key("minecraft:speed"),
                    0.5
                ),
                None
            );
        });
    }
}
