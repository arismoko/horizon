//! Server-side `PlayerRef` post-effect methods (`ServerPlayer.{get,add,remove,clear}PostEffects`).

use super::native_guest_turn_tests::with_guests;
use mod_api::{Identifier, PlayerWorldRefusal, prelude::*};
use std::sync::{Arc, mpsc::Receiver};
use sunrise_gameplay::game::ServerGame;
use sunrise_gameplay_common::game::execution_role::ExecutionRole;
use sunrise_protocol::{buf::Reader, packets::play::Clientbound as C};
use sunrise_sim::lan::{Out, PlayerSender};

fn id(text: &str) -> Identifier {
    Identifier::parse(text).unwrap()
}

fn listen(game: &mut ServerGame<'_>, conn: u32) -> Receiver<Out> {
    let (tx, rx) = std::sync::mpsc::channel();
    let hub = &mut game.server.lan.as_mut().unwrap().hub;
    hub.update_guest(conn, |g| g.sender = PlayerSender::new(tx, Arc::default()));
    rx
}

fn sent_post_effects(rx: &Receiver<Out>) -> Vec<Vec<String>> {
    rx.try_iter()
        .flat_map(|out| match out {
            Out::Packet(body) => vec![body],
            Out::Packets(bodies) => bodies,
            Out::Disconnect(_) => Vec::new(),
        })
        .filter_map(|body| {
            let mut reader = Reader::new(&body);
            let id = reader.varint().ok()?;
            match C::decode(id, reader.rest()).ok()? {
                C::PostEffects(packet) => Some(packet.effects),
                _ => None,
            }
        })
        .collect()
}

fn flush_dirty(game: &mut ServerGame<'_>, conn: u32) {
    let host = game.server.lan.as_mut().unwrap();
    if let Some(gi) = host.hub.guests.connection_index(conn)
        && host.hub.guests[gi].post_effects_dirty
    {
        host.hub.send_post_effects(gi);
    }
}

fn is_dirty(game: &ServerGame<'_>, conn: u32) -> bool {
    game.server
        .lan
        .as_ref()
        .unwrap()
        .hub
        .guest(conn)
        .unwrap()
        .post_effects_dirty
}

#[test]
#[ignore = "daily"]
fn post_effects_crud_preserves_order_and_syncs_only_target_guest() {
    with_guests(|game| {
        let players = game.players().server_players();
        let (a, b) = (players[0], players[1]);
        let (to_a, to_b) = (listen(game, 12), listen(game, 13));

        // Clear initial join dirty flag so we observe exact dirty transitions.
        flush_dirty(game, 12);
        flush_dirty(game, 13);
        let _ = sent_post_effects(&to_a);
        let _ = sent_post_effects(&to_b);
        assert!(!is_dirty(game, 12));
        assert!(!is_dirty(game, 13));

        assert_eq!(a.post_effects(game).unwrap(), Vec::<Identifier>::new());
        assert!(!a.clear_post_effects(game).unwrap());
        assert!(!is_dirty(game, 12));

        let invert = id("minecraft:invert");
        let custom = id("example:thermal");
        assert!(a.add_post_effect(game, &invert).unwrap());
        assert!(is_dirty(game, 12));
        assert!(!is_dirty(game, 13));

        // Adding a duplicate is a no-op and does not re-dirty when clean.
        flush_dirty(game, 12);
        assert_eq!(
            sent_post_effects(&to_a),
            vec![vec!["minecraft:invert".to_owned()]]
        );
        assert!(sent_post_effects(&to_b).is_empty());
        assert!(!a.add_post_effect(game, &invert).unwrap());
        assert!(!is_dirty(game, 12));

        // Second effect preserves insertion order.
        assert!(a.add_post_effect(game, &custom).unwrap());
        assert_eq!(
            a.post_effects(game).unwrap(),
            vec![invert.clone(), custom.clone()]
        );
        assert_eq!(b.post_effects(game).unwrap(), Vec::<Identifier>::new());

        // Removing an absent effect is a no-op; removing a present effect marks dirty.
        flush_dirty(game, 12);
        assert_eq!(
            sent_post_effects(&to_a),
            vec![vec![
                "minecraft:invert".to_owned(),
                "example:thermal".to_owned()
            ]]
        );
        assert!(!a.remove_post_effect(game, &id("example:missing")).unwrap());
        assert!(!is_dirty(game, 12));
        assert!(a.remove_post_effect(game, &invert).unwrap());
        assert!(is_dirty(game, 12));
        assert_eq!(a.post_effects(game).unwrap(), vec![custom]);

        // Clearing removes remaining effects and marks dirty; second clear is a no-op.
        flush_dirty(game, 12);
        assert_eq!(
            sent_post_effects(&to_a),
            vec![vec!["example:thermal".to_owned()]]
        );
        assert!(a.clear_post_effects(game).unwrap());
        assert!(is_dirty(game, 12));
        assert_eq!(a.post_effects(game).unwrap(), Vec::<Identifier>::new());
        flush_dirty(game, 12);
        assert_eq!(sent_post_effects(&to_a), vec![Vec::<String>::new()]);
        assert!(!a.clear_post_effects(game).unwrap());
        assert!(!is_dirty(game, 12));
        assert!(sent_post_effects(&to_b).is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn post_effects_reach_acting_and_displaced_players_and_refuse_stale_or_client() {
    with_guests(|game| {
        let players = game.players().server_players();
        let (a, b) = (players[0], players[1]);
        let effect_a = id("example:acting");
        let effect_b = id("example:displaced");

        game.as_guest(12, |game| {
            assert_eq!(game.players().acting_player(), Some(a));
            assert!(a.add_post_effect(game, &effect_a).unwrap());
            assert!(b.add_post_effect(game, &effect_b).unwrap());
            assert_eq!(a.post_effects(game).unwrap(), vec![effect_a.clone()]);
            assert_eq!(b.post_effects(game).unwrap(), vec![effect_b.clone()]);
        })
        .unwrap();

        assert_eq!(a.post_effects(game).unwrap(), vec![effect_a.clone()]);
        assert_eq!(b.post_effects(game).unwrap(), vec![effect_b]);

        // Respawning players (placed = false, respawning = true, as during player_clone)
        // remain accessible before hub.respawn sends ClientboundPostEffectsPacket.
        let respawn_effect = id("example:respawn");
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(12, |g| {
                g.placed = false;
                g.respawning = true;
            })
            .unwrap();
        assert!(a.add_post_effect(game, &respawn_effect).unwrap());
        assert_eq!(
            a.post_effects(game).unwrap(),
            vec![effect_a.clone(), respawn_effect.clone()]
        );
        assert!(a.remove_post_effect(game, &respawn_effect).unwrap());
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(12, |g| {
                g.placed = true;
                g.respawning = false;
            })
            .unwrap();

        // Stale epoch, mismatched uuid/entity, and offline handles refuse with Stale.
        let stale = mod_api::__private::game::player_ref(
            mod_api::EntityRef::new(
                mod_api::WorldEpoch::new(a.entity().epoch().get() + 1),
                a.entity().id(),
            ),
            a.uuid(),
        );
        let wrong_uuid = mod_api::__private::game::player_ref(a.entity(), 99);
        let wrong_entity = mod_api::__private::game::player_ref(b.entity(), a.uuid());
        for invalid in [stale, wrong_uuid, wrong_entity] {
            assert_eq!(invalid.post_effects(game), Err(PlayerWorldRefusal::Stale));
            assert_eq!(
                invalid.add_post_effect(game, &effect_a),
                Err(PlayerWorldRefusal::Stale)
            );
            assert_eq!(
                invalid.remove_post_effect(game, &effect_a),
                Err(PlayerWorldRefusal::Stale)
            );
            assert_eq!(
                invalid.clear_post_effects(game),
                Err(PlayerWorldRefusal::Stale)
            );
        }

        // Client role refuses with Client (and ClientGame does not implement PlayerSessionBackend).
        let saved_role = game.options.execution_role;
        game.options.execution_role = ExecutionRole::Client;
        assert_eq!(a.post_effects(game), Err(PlayerWorldRefusal::Client));
        assert_eq!(
            a.add_post_effect(game, &effect_a),
            Err(PlayerWorldRefusal::Client)
        );
        assert_eq!(
            a.remove_post_effect(game, &effect_a),
            Err(PlayerWorldRefusal::Client)
        );
        assert_eq!(a.clear_post_effects(game), Err(PlayerWorldRefusal::Client));
        game.options.execution_role = saved_role;

        // Disconnected/cleared guests refuse with Stale.
        game.server.lan.as_mut().unwrap().hub.guests.clear();
        assert_eq!(a.post_effects(game), Err(PlayerWorldRefusal::Stale));
        assert_eq!(
            a.add_post_effect(game, &effect_a),
            Err(PlayerWorldRefusal::Stale)
        );
    });
}
