#![cfg(test)]
#[cfg(feature = "client")]
use sunrise_core::geometry::Vec3;
#[cfg(feature = "client")]
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

#[cfg(test)]
pub(crate) mod tests {
    #[cfg(feature = "client")]
    use super::*;
    #[cfg(feature = "client")]
    use sunrise_gameplay_common::game::remote::general::tests::Declare;
    #[cfg(feature = "client")]
    use sunrise_gameplay_common::game::remote::general::tests::spawn;

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn a_shown_pickable_general_row_receives_the_named_use_and_attack() {
        use mod_api::{Flow, HostSide, InteractionResult};
        use std::sync::{Arc, Mutex};
        use sunrise_gameplay::game::native_player_death::tests::add_guest;
        use sunrise_protocol::packets::play::Serverbound as S;
        use {
            crate::game::client_side::native_hotbar_slot::hotbar_slot_tests::logged_in,
            crate::game::client_side::native_hotbar_slot::hotbar_slot_tests::sent,
        };
        use {
            crate::game::server_side::native_entity_refs::tests::with_client_game,
            sunrise_gameplay::game::native_entity_refs::tests::with_game,
        };
        use {
            sunrise_gameplay_common::game::general::GeneralEntityRecord,
            sunrise_gameplay_common::game::general::GeneralTypeEnvelope,
        };
        use {
            sunrise_level::chunk::Chunk, sunrise_sim::first_person_sim::Hand,
            sunrise_sim::motion::PlayerMotion,
        };

        const PACKAGE: &str = "mymod:package";
        const AT: [f64; 3] = [0.5, 65.0, 2.5];

        #[derive(Debug, PartialEq)]
        enum Event {
            ClientUse,
            ServerUse,
            Attack,
        }
        type Events = Arc<Mutex<Vec<(Event, i32, String)>>>;
        struct Watch(Events);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Watch {
            fn entity_interact(
                &self,
                host: HostSide<'_>,
                event: &mut mod_api::EntityInteractEvent<'_>,
            ) -> Option<InteractionResult> {
                let side = if matches!(host, HostSide::Client(_)) {
                    Event::ClientUse
                } else {
                    Event::ServerUse
                };
                self.0
                    .lock()
                    .unwrap()
                    .push((side, event.entity, event.kind.to_owned()));
                Some(InteractionResult::Consume)
            }

            fn attack_entity(
                &self,
                _: &mut dyn mod_api::ServerHost,
                event: &mut mod_api::AttackEntityEvent<'_>,
            ) -> Flow {
                self.0
                    .lock()
                    .unwrap()
                    .push((Event::Attack, event.entity, event.kind.to_owned()));
                Flow::Continue
            }
        }

        mod_platform::__private::initialize().unwrap();
        let _registration = mod_platform::__private::RegistrationGuard::open();
        let calls = Events::default();
        let mods = vec![
            mod_api::ModEntry::new("mymod", Box::new(Declare)),
            mod_api::ModEntry::new("observer", Box::new(Watch(Arc::clone(&calls)))),
        ];
        mod_platform::testing::with_mods(mods.into(), || {
            with_client_game(|game| {
                let mut pipe = logged_in(game);
                // A closer non-pickable row meets the ray too; the pickable row behind it
                // is immediately visible and pickable upon spawn without any attachment_sync.
                game.handle_packet(spawn(40, "mymod:missile", [0.5, 66.5, 1.2]))
                    .unwrap();
                assert_eq!(
                    game.entity(40).unwrap().piston_push_reaction(game),
                    Ok(Some(mod_api::PushReaction::IgnoreEntity))
                );
                game.handle_packet(spawn(42, PACKAGE, AT)).unwrap();
                game.refresh_client_pick(1.0);
                assert!(
                    matches!(game.stored_client_pick(), Some(mod_api::CrosshairTarget::Entity { entity, .. }) if entity.id() == 42)
                );
                game.remote_use(mod_api::InteractionAttempt::Press);
                game.remote_start_attack();
                let packets: Vec<_> = sent(&mut pipe)
                    .into_iter()
                    .filter_map(|packet| match packet {
                        S::Interact(packet) => Some((Event::ClientUse, packet.id)),
                        S::Attack(packet) => Some((Event::Attack, packet.id)),
                        _ => None,
                    })
                    .collect();
                assert_eq!(packets, [(Event::ClientUse, 42), (Event::Attack, 42)]);
                assert_eq!(
                    *calls.lock().unwrap(),
                    [(Event::ClientUse, 42, PACKAGE.into())]
                );
            });
            calls.lock().unwrap().clear();
            with_game(|game| {
                let record = GeneralEntityRecord {
                    position: AT,
                    velocity: [0.0; 3],
                    rotation: [0.0; 2],
                    owner: None,
                    custom_name: None,
                    custom_name_visible: false,
                    base: Default::default(),
                    on_ground: false,
                    fall_distance: 0.0,
                    uuid: [1, 0, 0, 4],
                    envelope: GeneralTypeEnvelope {
                        type_key: PACKAGE.into(),
                        payload_version: 1,
                        payload_bytes: Vec::new(),
                    },
                    passengers: Vec::new(),
                };
                game.load_listed(PACKAGE, None, None, record.to_tag())
                    .unwrap();
                let id = game.resident_entities.general.rows[0].id as i32;
                add_guest(game);
                let at = Vec3::new(0.5, 65.0, 0.5);
                let mut body = PlayerMotion::new(game.motion, at).unwrap();
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| {
                        guest.position = at.components();
                        guest.load_timeout = 0;
                        body.entity_id = guest.entity_id;
                        guest.motion = Some(Box::new(body));
                    });
                game.reset_entity_residency();
                assert_eq!(
                    game.guest_interact(1, 0, id, Hand::Main, [0.0, 1.65, -0.25]),
                    Ok(true)
                );
                // The named row is still in packet range, but a fresh ray can't see it.
                if !game.world.chunks.contains_key(&(0, 0)) {
                    game.world.chunks.insert((0, 0), Chunk::empty(0, 0).into());
                }
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.set(0, 65, 1, stone).unwrap();
                game.world.set(0, 66, 1, stone).unwrap();
                game.as_guest(1, |guest| guest.server.attack_ticks = 20)
                    .unwrap();
                assert_eq!(
                    game.guest_attack(1, 0, id),
                    Ok(false),
                    "an unclaimed hit cannot hurt a plain row"
                );
                assert_eq!(game.as_guest(1, |guest| guest.server.attack_ticks), Some(0));
                assert!(game.resident_entities.general.row(id as u32).is_some());
                game.as_guest(1, |guest| guest.player.as_mut().unwrap().position.z = -64.5)
                    .unwrap();
                let too_far = Err(mod_api::ActionRefused::OutOfReach);
                assert_eq!(
                    game.guest_interact(1, 0, id, Hand::Main, [0.0, 1.65, -0.25]),
                    too_far
                );
                assert_eq!(game.guest_attack(1, 0, id), too_far);
                assert_eq!(
                    *calls.lock().unwrap(),
                    [
                        (Event::ServerUse, id, PACKAGE.into()),
                        (Event::Attack, id, PACKAGE.into())
                    ]
                );
                game.persist.removals.push(
                    sunrise_gameplay_common::game::commands_common::Target::General(id as u32),
                );
                assert_eq!(
                    game.guest_attack(1, 0, id),
                    Err(mod_api::ActionRefused::InvalidTarget),
                    "the aim fallback must not recover a pending removal"
                );
            });
        });
    }
}
