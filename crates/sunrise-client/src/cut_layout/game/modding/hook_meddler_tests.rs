//! A test mod whose listeners change the world while the game is inside a hook, for the tests of
//! the game's check after a hook that can change the world: once armed, it runs one action at the
//! first of the hooks it listens to, and does not cancel.
use super::native_entity_refs::tests::with_game;
use mod_api::{
    ArmorStandManipulateEvent, EntityMountEvent, Flow, HangingBreakEvent, Hooks,
    PlayerShearEntityEvent, ServerHost,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

type Action = Box<dyn Fn(&mut dyn ServerHost) + Send + Sync>;

/// Whether the meddler acts at the next hook it hears (`armed`), and whether it has (`fired`). A
/// test sets its scene up unarmed: the commands that do it fire hooks too.
#[derive(Default)]
pub(crate) struct Gate {
    armed: AtomicBool,
    fired: AtomicBool,
}
impl Gate {
    pub(crate) fn arm(&self) {
        self.armed.store(true, Ordering::SeqCst);
    }
    pub(crate) fn fired(&self) -> bool {
        self.fired.load(Ordering::SeqCst)
    }
}

struct Meddler {
    act: Action,
    gate: Arc<Gate>,
}
impl Meddler {
    fn meddle(&self, host: &mut dyn ServerHost) -> Flow {
        if self.gate.armed.load(Ordering::SeqCst) && !self.gate.fired.swap(true, Ordering::SeqCst) {
            (self.act)(host);
        }
        Flow::Continue
    }
}
#[mod_api::__subscriptions]
impl Hooks for Meddler {
    fn armor_stand_manipulate(
        &self,
        host: &mut dyn ServerHost,
        _: &mut ArmorStandManipulateEvent<'_>,
    ) -> Flow {
        self.meddle(host)
    }
    fn player_shear_entity(
        &self,
        host: &mut dyn ServerHost,
        _: &mut PlayerShearEntityEvent<'_>,
    ) -> Flow {
        self.meddle(host)
    }
    fn hanging_break(&self, host: &mut dyn ServerHost, _: &mut HangingBreakEvent<'_>) -> Flow {
        self.meddle(host)
    }
    fn entity_mount(&self, host: &mut dyn ServerHost, _: &mut EntityMountEvent<'_>) -> Flow {
        self.meddle(host)
    }
}

/// Runs `test` on a server game with the meddling mod, which does `act` the first time a hook it
/// listens to fires once the gate is armed.
pub(crate) fn with_meddler(
    act: impl Fn(&mut dyn ServerHost) + Send + Sync + 'static,
    test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, &Gate),
) {
    let gate = Arc::new(Gate::default());
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "meddler",
        Box::new(Meddler {
            act: Box::new(act),
            gate: gate.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            // A server hears these hooks.
            game.options.dedicated = Some(Vec::new());
            // The command service requires an open server, not just dedicated options.
            game.lan_host(
                sunrise_gameplay::game::lan::LanSettings {
                    port: 0,
                    guest_commands: false,
                    force_game_mode: false,
                },
                sunrise_sim::lan::NetConfig {
                    listen: false,
                    announce: false,
                    lod_distance: 0,
                    ..sunrise_sim::lan::NetConfig::lan(0, String::new(), 2)
                },
                sunrise_sim::lan::hub::HubConfig {
                    default_game_mode: 0,
                    guest_commands: false,
                    force_game_mode: false,
                    hardcore: false,
                    view_distance: 2,
                    simulation_distance: 2,
                    dimension: game.dim.id().to_owned(),
                    allow_flight: true,
                    seed: 0,
                    max_players: 2,
                    integrated: true,
                    levels: game.server.level_shapes.clone(),
                    level_seeds: game.server.levels.0.runtime.seeds(),
                },
            )
            .unwrap();
            // Scene-building commands need an operator source: dedicated options do not
            // grant permission, and this fixture has not opened a world with Allow Commands.
            game.set_allow_commands(true);
            game.build_registries();
            test(game, &gate);
        })
    });
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
