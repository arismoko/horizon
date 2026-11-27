use crate::game::server_side::wind::wind_charged_tests::BREEZE_BURST;
use crate::game::server_side::wind::wind_charged_tests::WIND_CHARGE_BURST;
use crate::game::server_side::wind::wind_charged_tests::a_wind_charged_zombie_dies;
use crate::game::server_side::wind::wind_charged_tests::boat;
use crate::game::server_side::wind::wind_charged_tests::boat_speed;
use crate::game::server_side::wind::wind_charged_tests::with_game;
use crate::game::server_side::wind::wind_charged_tests::*;

use std::{path::Path, sync::Arc};
use sunrise_protocol::registry::Builtin;
use sunrise_sim::lan::data::SyncData;

const SMALL_GUST: &str = "minecraft:gust_emitter_small";
const LARGE_GUST: &str = "minecraft:gust_emitter_large";

fn gust_sync(resources: &Path) -> Arc<SyncData> {
    let mut sync = SyncData::empty();
    sync.builtin = Builtin::load(&Builtin::find(resources).unwrap()).unwrap();
    Arc::new(sync)
}

/// Count `Clientbound::Explode` packets carrying emitter particle id `particle_id`.
fn gusts(rx: &std::sync::mpsc::Receiver<sunrise_sim::lan::Out>, particle_id: i32) -> usize {
    use sunrise_protocol::{buf::Reader, packets::play};
    use sunrise_sim::lan::Out;
    let mut count = 0;
    while let Ok(out) = rx.try_recv() {
        let Out::Packet(body) = out else {
            continue;
        };
        let mut r = Reader::new(&body);
        let Ok(id) = r.varint() else {
            continue;
        };
        if let Ok(play::Clientbound::Explode(p)) = play::Clientbound::decode(id, r.rest())
            && p.particle.id == particle_id
        {
            count += 1;
        }
    }
    count
}

/// `AbstractWindCharge.explode` hands `ServerLevel.explode` the charge itself
/// as the source, so `playSound` is `!charge.isSilent()`: a `Silent` wind
/// charge's burst is mute, its gust and its push stay. The sound is the wind
/// charge's own, as `WindCharge.explode` passes it.
#[test]
#[ignore = "daily"]
fn a_silent_wind_charge_bursts_without_a_sound() {
    let Some(outcome) = with_game(|game| {
        let (hub, near, _far) = sunrise_gameplay::game::server_side::lan_particles::tests::test_hub(
            "WindChargeSilentTest",
            Vec::new(),
            Some(gust_sync(&game.options.resources)),
        );
        let gust_id = hub
            .registry_id("minecraft:particle_type", SMALL_GUST)
            .unwrap();
        game.server.lan = Some(sunrise_gameplay::game::lan::LanHost::for_test(hub));
        let mut outcome = Vec::new();
        for silent in [false, true] {
            let boat = boat(game);
            let (charge, breeze) = (heard(game, WIND_CHARGE_BURST), heard(game, BREEZE_BURST));
            let nbt = if silent {
                "{Silent:1b,Motion:[0.0,-1.0,0.0]}"
            } else {
                "{Motion:[0.0,-1.0,0.0]}"
            };
            game.summon_with_nbt("minecraft:wind_charge", [4.5, 66.0, 4.5], nbt)
                .unwrap()
                .expect("summoned wind charge");
            for _ in 0..40 {
                game.tick_creatures().unwrap();
            }
            outcome.push((
                silent,
                heard(game, WIND_CHARGE_BURST) - charge,
                heard(game, BREEZE_BURST) - breeze,
                gusts(&near, gust_id),
                boat_speed(game, boat) > 0.0,
            ));
            game.boats.clear();
        }
        game.server.lan = None;
        outcome
    }) else {
        return;
    };
    assert_eq!(
        outcome,
        vec![
            // Heard once, as the wind charge's, with the gust and the boat thrown.
            (false, 1, 0, 1, true),
            // A silent charge's burst is mute, and still pushes and draws.
            (true, 0, 0, 1, true),
        ]
    );
}

/// `WindChargedMobEffect.onMobRemoved` calls `ServerLevel.explode` with
/// `SoundEvents.BREEZE_WIND_CHARGE_BURST`, so the burst is heard as the
/// breeze's, not as the wind charge's; `ServerLevel.explode` sends
/// `playSound = source == null || !source.isSilent()`, so a `Silent` mob's
/// burst is mute. The particle and the knockback stay either way.
#[test]
#[ignore = "daily"]
fn a_wind_charged_mob_bursts_with_the_breezes_sound_unless_it_is_silent() {
    let Some(outcome) = with_game(|game| {
        let (hub, near, _far) = sunrise_gameplay::game::server_side::lan_particles::tests::test_hub(
            "WindChargedMobTest",
            Vec::new(),
            Some(gust_sync(&game.options.resources)),
        );
        let gust_id = hub
            .registry_id("minecraft:particle_type", LARGE_GUST)
            .unwrap();
        game.server.lan = Some(sunrise_gameplay::game::lan::LanHost::for_test(hub));
        let mut outcome = Vec::new();
        for silent in [false, true] {
            let boat = boat(game);
            let (breeze, charge) = (heard(game, BREEZE_BURST), heard(game, WIND_CHARGE_BURST));
            a_wind_charged_zombie_dies(game, silent);
            outcome.push((
                silent,
                heard(game, BREEZE_BURST) - breeze,
                heard(game, WIND_CHARGE_BURST) - charge,
                gusts(&near, gust_id),
                boat_speed(game, boat) > 0.0,
            ));
            game.boats.clear();
        }
        game.server.lan = None;
        outcome
    }) else {
        return;
    };
    assert_eq!(
        outcome,
        vec![
            // Heard once, as the breeze's, with the gust and the boat thrown.
            (false, 1, 0, 1, true),
            // A silent mob's burst is mute, and still pushes and draws.
            (true, 0, 0, 1, true),
        ]
    );
}
