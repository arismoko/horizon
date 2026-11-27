//! After `entity_mount` lets a mob board a boat it touches, the game finds the boat and the mob
//! again by id, counts the boat's passengers again and boards the mob only if it is still free.
use crate::game::server_side::hook_meddler_tests::with_meddler;
use sunrise_core::position::ChunkPos;
use sunrise_gameplay::game::native_entity_refs::tests::prepare_entity_ticking_neighborhood;

fn mob_id(game: &sunrise_gameplay::game::ServerGame<'_>, kind: &str) -> u32 {
    game.mobs.iter().find(|m| &*m.kind.id == kind).unwrap().id
}

/// An empty boat and a pig touching it, both held still, and the boat's id.
fn set_up(game: &mut sunrise_gameplay::game::ServerGame<'_>) -> u32 {
    assert!(game.run_command("summon oak_boat 8.5 100 8.5"));
    assert!(game.run_command("summon pig 8.5 100 8.5 {NoAI:1b,Silent:1b}"));
    game.boats[0].id
}

#[test]
#[ignore = "daily"]
fn a_boat_the_listener_removed_ends_the_contact_pass() {
    with_meddler(
        |host| {
            assert!(
                host.commands()
                    .run_command("kill @e[type=minecraft:oak_boat]")
            );
        },
        |game, gate| {
            set_up(game);
            gate.arm();
            game.boat_contacts(0);
            assert!(gate.fired());
            assert!(game.boats.is_empty());
            assert_eq!(game.mobs[0].extra.mount, None, "the pig boarded nothing");
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_boat_the_listener_filled_skips_the_mob() {
    with_meddler(
        |host| {
            let seat = |kind: &str| {
                format!(
                    "ride @e[type=minecraft:{kind},limit=1] mount @e[type=minecraft:oak_boat,limit=1]"
                )
            };
            assert!(host.commands().run_command(&seat("cow")));
            assert!(host.commands().run_command(&seat("sheep")));
        },
        |game, gate| {
            prepare_entity_ticking_neighborhood(game, ChunkPos::new(0, 0));
            let boat = set_up(game);
            assert!(game.run_command("summon cow 40.5 100 40.5 {NoAI:1b,Silent:1b}"));
            assert!(game.run_command("summon sheep 40.5 100 40.5 {NoAI:1b,Silent:1b}"));
            gate.arm();
            game.boat_contacts(0);
            assert!(gate.fired());
            assert_eq!(game.seated_passenger_count(boat), 2, "a boat seats two");
            let pig = mob_id(game, "minecraft:pig");
            assert_eq!(game.mob_object(pig).unwrap().extra.mount, None);
        },
    );
}
