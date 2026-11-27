//! ClientLevel.animateTick lends the level random to each block part before that pick's fluid runs.
#[cfg(feature = "client")]
use mod_api::ModEntry;
#[cfg(feature = "client")]
use mod_api::{
    AnimateTickEvent, BlockPart, BlockPos, ClientHost, Hooks, ParticleOptions, Registrar,
    RegistrationContext,
};
#[cfg(feature = "client")]
use std::sync::{Arc, Mutex};
#[cfg(feature = "client")]
use sunrise_core::geometry::Vec3;
#[cfg(feature = "client")]
use sunrise_level::chunk::Chunk;
#[cfg(feature = "client")]
use sunrise_sim::motion::PlayerMotion;
#[cfg(feature = "client")]
use {
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_client_gameplay::game::client_side::native_client_session::tests::logged_client,
};

/// One call the part heard: its position and block, its draw before and after its particle,
/// and whether the particle was submitted.
#[cfg(feature = "client")]
pub(crate) type Call = (BlockPos, String, f32, f32, bool);
#[cfg(feature = "client")]
pub(crate) type Calls = Arc<Mutex<Vec<Call>>>;

/// Draws, adds a heart at the block's centre, draws again.
#[cfg(feature = "client")]
pub(crate) struct Hearts {
    pub calls: Calls,
    pub(crate) heart: mod_api::ParticleKey,
}
#[cfg(feature = "client")]
#[mod_api::sunrise_part]
impl BlockPart for Hearts {
    fn animate_tick(&self, host: &mut dyn ClientHost, event: &mut AnimateTickEvent<'_>) {
        let first = event.random.next_float();
        let p = event.pos;
        let at = [
            f64::from(p.x) + 0.5,
            f64::from(p.y) + 0.5,
            f64::from(p.z) + 0.5,
        ];
        let added = host.add_particle(&ParticleOptions::simple(self.heart), at, [0.0; 3]);
        let second = event.random.next_float();
        let block = event.state.block().id(&*host).unwrap().to_owned();
        self.calls
            .lock()
            .unwrap()
            .push((p, block, first, second, added));
    }
}
#[cfg(feature = "client")]
pub(crate) struct Register(pub(crate) Calls);
#[cfg(feature = "client")]
#[mod_api::__subscriptions]
impl Hooks for Register {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let heart = r.particle_key("minecraft:heart");
        for block in ["minecraft:glass", "minecraft:lightning_rod"] {
            let block = r.block_key(block);
            r.block_part(
                block,
                Hearts {
                    calls: self.0.clone(),
                    heart,
                },
            );
        }
    }
}

/// A client game with the hearts part installed, in loaded empty chunks around a player at
/// `(8.5, 65, 8.5)`.
#[cfg(feature = "client")]
pub(crate) fn with_hearts(
    test: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>, &Calls),
) {
    let calls = Calls::default();
    let mods = vec![ModEntry::new("hearts", Box::new(Register(calls.clone())))];
    mod_platform::testing::with_mods(mods.into(), || {
        with_client_game(|game| {
            let _server = logged_client(game);
            for cx in -2..=2 {
                for cz in -2..=2 {
                    game.world
                        .chunks
                        .insert((cx, cz), Arc::new(Chunk::empty(cx, cz)));
                }
            }
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(8.5, 65.0, 8.5)).unwrap());
            test(game, &calls);
        })
    });
}
