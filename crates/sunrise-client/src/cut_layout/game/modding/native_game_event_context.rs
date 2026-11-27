#![cfg(test)]

#[cfg(test)]
mod tests {
    use mod_api::{EntityRef, GameEventContext};
    use std::sync::Arc;
    use sunrise_game_data::who::EventSource;
    use sunrise_gameplay::game::native_entity_refs::tests::with_game;
    use sunrise_gameplay_common::game::api_values::TryIntoApi;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_items::items::ItemStack;
    use sunrise_level::chunk::Chunk;

    fn sensor<'a>(
        game: &'a mut sunrise_gameplay::game::ServerGame<'_>,
    ) -> Option<&'a mut sunrise_level::sculk::SculkEntity> {
        match game.world.block_entities.get_mut(&(1, 64, 1)) {
            Some(sunrise_level::containers::BlockEntity::Sculk(entity)) => Some(entity),
            _ => None,
        }
    }

    #[test]
    #[ignore = "daily"]
    fn contextual_game_event_keeps_guest_cause_until_emission() {
        use std::sync::Mutex;
        use sunrise_gameplay::game::server_side::native_item_places::tests::open_to_lan;
        use sunrise_gameplay_common::game::server_side::native_sound_refs::tests::guest;
        struct Heard(Arc<Mutex<Vec<Option<u32>>>>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Heard {
            fn vanilla_game_event(
                &self,
                _: &mut dyn mod_api::ServerHost,
                event: &mut mod_api::VanillaGameEventEvent<'_>,
            ) -> mod_api::Flow {
                self.0.lock().unwrap().push(event.cause);
                mod_api::Flow::Continue
            }
        }
        let heard = Arc::new(Mutex::new(Vec::new()));
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new(
                "guest_events",
                Box::new(Heard(heard.clone())),
            )]
            .into(),
            || {
                with_game(|game| {
                    open_to_lan(game);
                    let (guest, _receiver) = guest(7, 99, [2.5, 64.0, 1.5]);
                    let id = guest.entity_id as u32;
                    game.server.lan.as_mut().unwrap().hub.guests.push(guest);
                    let source = game.entity(id).unwrap();
                    let level = game.level(game.dim.id()).unwrap();
                    heard.lock().unwrap().clear();
                    assert!(game.server.lan_acting_entity.is_none());
                    assert!(game.level_game_event(
                        level,
                        "minecraft:step",
                        [3.5, 64.0, 1.5],
                        GameEventContext {
                            source_entity: Some(source),
                            affected_state: None
                        }
                    ));
                    assert!(
                        game.server.lan_acting_entity.is_none(),
                        "guest scope must be restored"
                    );
                    assert_eq!(*heard.lock().unwrap(), [Some(id)]);
                });
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn sensor_receives_context_source_and_affected_state_at_requested_position() {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.sculk_prepare();
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let registry = &game.pack.assets.registry;
            let sensor_state = registry.default_state("minecraft:sculk_sensor").unwrap();
            let stone = registry
                .default_state("minecraft:stone")
                .unwrap()
                .try_into_api()
                .unwrap();
            let wool = registry
                .default_state("minecraft:white_wool")
                .unwrap()
                .try_into_api()
                .unwrap();
            game.world.set(1, 64, 1, sensor_state).unwrap();
            game.sculk_sync_index();
            game.spawn_item(
                ItemStack::new("minecraft:stone", 1),
                [12.5, 64.0, 1.5],
                [0.0; 3],
                0,
            );
            let source = game.entity(game.dropped[0].id).unwrap();
            let level = game.level(game.dim.id()).unwrap();
            let at = [3.5, 64.0, 1.5];
            assert!(game.level_game_event(
                level,
                "minecraft:block_change",
                at,
                GameEventContext {
                    source_entity: Some(source),
                    affected_state: Some(stone)
                }
            ));
            let vibration = &mut sensor(game).unwrap().vibration;
            let (info, _) = vibration.selector.current.take().unwrap();
            assert_eq!(info.source, Some(EventSource::Entity(source.id())));
            assert_eq!(
                info.pos, at,
                "the explicit event position must not become the source position"
            );
            assert!(game.level_game_event(
                level,
                "minecraft:block_change",
                at,
                GameEventContext {
                    source_entity: Some(source),
                    affected_state: Some(wool)
                }
            ));
            assert!(
                sensor(game).unwrap().vibration.selector.current.is_none(),
                "affected wool must dampen the vibration"
            );
            let stale = EntityRef::new(
                mod_api::WorldEpoch::new(source.epoch().get().wrapping_add(1)),
                source.id(),
            );
            assert!(!game.level_game_event(
                level,
                "minecraft:block_change",
                at,
                GameEventContext {
                    source_entity: Some(stale),
                    affected_state: None
                }
            ));
            assert!(sensor(game).unwrap().vibration.selector.current.is_none());
            assert!(game.level_game_event(
                level,
                "minecraft:block_change",
                at,
                GameEventContext::default()
            ));
            let (info, _) = sensor(game)
                .unwrap()
                .vibration
                .selector
                .current
                .take()
                .unwrap();
            assert_eq!(info.source, None);
        });
    }
}
