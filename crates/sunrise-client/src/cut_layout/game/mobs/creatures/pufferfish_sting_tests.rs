//! `Pufferfish.playerTouch`: the sting game event (`ClientboundGameEventPacket.PUFFER_FISH_STING`)
//! goes out only inside the `hurtServer` branch, and only when the fish is not
//! silent; the poison follows it.

#[allow(unused_imports)]
pub(crate) use crate::game::server_side::creatures_server::pufferfish_player::pufferfish_sting_tests::with_fish;

#[allow(unused_imports)]
pub(crate) use crate::game::server_side::creatures_server::pufferfish_player::pufferfish_sting_tests::stings;

pub(crate) fn health(game: &sunrise_gameplay::game::ServerGame<'_>) -> f32 {
    game.player.as_ref().unwrap().health.health()
}

#[test]
#[ignore = "daily"]
fn the_sting_event_goes_out_once_per_accepted_hit() {
    with_fish(false, |g| {
        g.touch_pufferfish();
        assert_eq!(health(g), 17.0, "1 + PuffState 2 of damage landed");
        assert_eq!(stings(g), vec![(9, 0.0)], "PUFFER_FISH_STING, parameter 0");
        let poison = g.potions.effects.get("poison").expect("poisoned");
        assert_eq!((poison.duration, poison.amplifier), (120, 0));
        // A second touch inside the hurt cooldown is refused (`hurtServer` is
        // false): no damage and no event.
        g.touch_pufferfish();
        assert_eq!(health(g), 17.0, "the cooldown refused the second hit");
        assert_eq!(stings(g).len(), 1, "a refused hit sends nothing");
    });
}

#[test]
#[ignore = "daily"]
fn a_silent_fish_damages_and_poisons_without_the_sting() {
    with_fish(true, |g| {
        g.touch_pufferfish();
        assert_eq!(health(g), 17.0, "the damage still lands");
        assert!(g.potions.effects.get("poison").is_some(), "the poison too");
        assert!(stings(g).is_empty(), "a silent fish sends no event");
    });
}
