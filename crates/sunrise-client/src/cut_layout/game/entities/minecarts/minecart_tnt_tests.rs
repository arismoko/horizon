//! `MinecartTNT`'s click and fuse: vanilla has no `interact` override, so
//! flint and steel leaves the cart alone (`Entity.interact` passes), and
//! `primeFuse` plays `entity.tnt.primed` on the blocks source only when the
//! cart is not silent.
use sunrise_gameplay::game::game_tests::tests::server_game;

/// A server's game on the prepared resources, or None when they are absent
/// (CI has none: the tests then skip quietly).
pub(crate) fn server(
    resources: &mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
) -> Option<sunrise_gameplay::game::ServerGame<'_>> {
    std::env::var_os("SUNRISE_TEST_RESOURCES")?;
    Some(server_game(resources))
}

#[allow(unused_imports)]
pub(crate) use sunrise_gameplay::game::server_side::minecarts_server::minecart_tnt_tests::tnt_cart;

#[test]
fn a_silent_tnt_cart_primes_without_its_sound() {
    let mut resources = None;
    let Some(mut game) = server(&mut resources) else {
        return;
    };
    // Silent: the fuse is lit, the entity event goes out, no sound is played.
    tnt_cart(&mut game, 42, true);
    game.start_audio_capture();
    let mut silent = game.carts[0].clone();
    game.activate_cart(&mut silent, true);
    assert_eq!(silent.fuse, 80, "a silent cart still primes");
    assert!(
        game.server
            .lan_entity_events
            .iter()
            .any(|(id, event)| *id == 42 && *event == 70),
        "a silent cart still broadcasts EntityEvent.TNT_PRIME: {:?}",
        game.server.lan_entity_events
    );
    let sounds = std::mem::take(&mut game.server.pending_sounds);
    game.stop_audio_capture();
    assert!(sounds.is_empty(), "a silent cart hissed: {sounds:?}");

    // Not silent: exactly one `entity.tnt.primed` at the cart, on the blocks
    // source (`SoundSource.BLOCKS` ordinal 4), volume and pitch 1.
    tnt_cart(&mut game, 43, false);
    game.start_audio_capture();
    let mut normal = game.carts[0].clone();
    game.activate_cart(&mut normal, true);
    assert_eq!(normal.fuse, 80);
    let sounds = std::mem::take(&mut game.server.pending_sounds);
    game.stop_audio_capture();
    assert_eq!(sounds.len(), 1, "{sounds:?}");
    let sound = &sounds[0];
    assert_eq!(sound.event, "minecraft:entity.tnt.primed");
    assert_eq!(sound.position, [0.5, 64.0, 0.5]);
    assert_eq!(sound.volume, 1.0);
    assert_eq!(sound.pitch, 1.0);
    assert_eq!(sound.source, 4, "SoundSource.BLOCKS");
}
