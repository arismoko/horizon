//! Headless proof of the generated Host bridge and the ordinary particle queue.
use crate::game::ClientGame;
use mod_api::ClientHost;
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};
use sunrise_client::{
    hud::Rect,
    particles::{ParticleEngine, ParticleLevel},
};
use sunrise_core::geometry::Vec3;
use sunrise_game_data::Result;
use sunrise_level::particle_options::{ParticleOptions, ParticleType};
use sunrise_sim::motion::PlayerMotion;

fn add(
    game: &mut crate::game::ClientGame<'_>,
    options: &ParticleOptions,
    at: [f64; 3],
    velocity: [f64; 3],
) -> bool {
    use sunrise_gameplay_common::game::api_values::TryIntoApi;
    let Ok(options) = options.try_into_api() else {
        return false;
    };
    let host: &mut dyn ClientHost = game;
    host.add_particle(&options, at, velocity)
}

impl ClientGame<'_> {
    fn particle_fixture_tick(&mut self) {
        let level = ParticleLevel {
            world: &self.common.world,
            facts: self.common.res.facts(),
            collision: self.common.collision,
            player: None,
            cached_chunk: std::cell::Cell::new(None),
        };
        self.client.particles.0.tick(&level);
    }

    pub(in crate::game::client_side) fn run_native_client_particles_fixture(
        &mut self,
        directory: &Path,
    ) -> Result<()> {
        // The fixture stands in for a joined client level, where the mods' calls run in the
        // World phase; the loading window it starts from is still in Setup.
        let at = [0.0, 100.0, 0.0];
        let smoke = ParticleOptions::simple(ParticleType::Smoke);
        self.client.particles.0.requested = Some(Vec::new());
        self.common.player = None;
        assert!(!add(self, &smoke, at, [0.0; 3]), "menu has no client level");
        self.common.player = Some(PlayerMotion::new(
            self.common.motion,
            Vec3::new(0.0, 100.0, 0.0),
        )?);
        let encoded = [f64::from(0xff12ab), 7.0, -2.0];
        let mut custom = ParticleOptions::named("fixture:encoded");
        custom.color = [0.1, 0.2, 0.3, 0.4];
        custom.scale = 2.5;
        assert!(add(self, &custom, at, encoded));
        let requests = self.client.particles.0.requested.take().unwrap();
        assert_eq!(requests.len(), 1, "menu and server guards queued nothing");
        assert_eq!(requests[0].0.color, custom.color);
        assert_eq!(requests[0].0.scale, custom.scale);
        assert_eq!(
            requests[0].0.mod_particle.as_deref(),
            Some("fixture:encoded")
        );
        assert_eq!(
            (requests[0].1, requests[0].2),
            (at, encoded),
            "provider slots must remain unchanged"
        );
        self.client.particles.0.requested = Some(Vec::new());
        for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(!add(self, &smoke, [invalid, 100.0, 0.0], [0.0; 3]));
            assert!(!add(self, &smoke, at, [0.0, invalid, 0.0]));
        }
        let mut invalid_options = smoke.clone();
        invalid_options.color[1] = f32::NAN;
        assert!(!add(self, &invalid_options, at, [0.0; 3]));
        invalid_options = smoke.clone();
        invalid_options.scale = f32::INFINITY;
        assert!(!add(self, &invalid_options, at, [0.0; 3]));

        assert!(
            self.client.particles.0.requested.take().unwrap().is_empty(),
            "invalid requests must not touch the queue"
        );

        // Own sprite metadata and atlas rectangles exercise real constructors and CPU quads,
        // without a GPU, downloaded art, a resource reload or a change to imported resources.
        let pack = directory.join("particle-pack");
        std::fs::create_dir_all(pack.join("assets/minecraft/particles"))?;
        std::fs::create_dir_all(pack.join("assets/fixture/particles"))?;
        std::fs::write(
            pack.join("assets/minecraft/particles/smoke.json"),
            br#"{"textures":["fixture_smoke"]}"#,
        )?;
        std::fs::write(pack.join("assets/fixture/particles/spark.json"), br#"{"textures":["fixture:spark"],"sunrise:behaviour":{"velocity":[0.25,0,0],"gravity":0,"friction":1,"lifetime":3,"size":[0.2,0.2],"animation":"first"}}"#)?;
        let rect = Rect {
            u0: 0.0,
            v0: 0.0,
            u1: 1.0,
            v1: 1.0,
            w: 16,
            h: 16,
        };
        self.client.particles.0 = ParticleEngine::new(
            &sunrise_assets::resource_source::ResourceSource::directory(pack.clone()),
            &HashMap::from([
                ("particle/fixture_smoke".into(), rect),
                ("fixture:particle/spark".into(), rect),
            ]),
            &HashSet::new(),
        );
        self.client
            .particles
            .0
            .set_block_state_source(super::native_client_particles::particle_block_states);
        self.client.particles.0.camera = at;
        assert!(add(self, &smoke, at, [0.0; 3]));
        assert_eq!(
            self.client.particles.0.len(),
            0,
            "engine defers pending spawns until its tick"
        );
        self.particle_fixture_tick();
        assert_eq!(
            self.client.particles.0.len(),
            1,
            "vanilla provider reached the live queue"
        );
        let mut vertices = Vec::new();
        self.client.particles.0.render(
            glam::DVec3::from_array(at),
            glam::Quat::IDENTITY,
            1.0,
            &|_| 15,
            &mut vertices,
        );
        assert_eq!(
            vertices.len(),
            4,
            "vanilla particle reaches ordinary render data"
        );
        self.client.particles.0.clear();
        assert!(add(self, &smoke, [64.0, 100.0, 0.0], [0.0; 3]));
        self.particle_fixture_tick();
        assert!(
            self.client.particles.0.is_empty(),
            "ordinary camera-distance limiter is retained"
        );
        self.client.particles.0.particle_level = 2;
        assert!(add(self, &smoke, at, [0.0; 3]));
        self.particle_fixture_tick();
        assert!(
            self.client.particles.0.is_empty(),
            "ordinary minimal setting is retained"
        );
        self.client.particles.0.particle_level = 0;
        assert!(add(
            self,
            &ParticleOptions::named("fixture:spark"),
            at,
            [0.0; 3]
        ));
        self.particle_fixture_tick();
        assert_eq!(
            self.client.particles.0.len(),
            1,
            "existing named provider reached the live queue"
        );
        self.particle_fixture_tick();
        vertices.clear();
        self.client.particles.0.render(
            glam::DVec3::from_array(at),
            glam::Quat::IDENTITY,
            1.0,
            &|_| 15,
            &mut vertices,
        );
        assert_eq!(vertices.len(), 4);
        assert!(vertices.iter().flat_map(|v| v.pos).all(f32::is_finite));
        let center_x = vertices.iter().map(|v| v.pos[0]).sum::<f32>() / 4.0;
        assert!(
            (center_x - 0.25).abs() < 1e-6,
            "named behavior moved the real particle: {center_x}"
        );
        for _ in 0..10 {
            self.particle_fixture_tick();
        }
        assert!(
            self.client.particles.0.is_empty(),
            "named provider lifetime still expires normally"
        );
        std::fs::write(
            directory.join("report.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "client_spawn": true, "encoded_slots_unchanged": true,
                "menu_and_servers_refused": true, "nonfinite_refused": true,
                "vanilla_live_particles": 1, "vanilla_vertices": 4,
                "named_live_particles": 1, "named_vertices": 4, "named_tick_x": center_x,
                "named_expired": true, "normal_limiters_retained": true,
                "scope": "Generated Host bridge into existing particle queue, providers, tick and CPU render data; no GPU/window or new providers/triggers."
            }))?,
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{
        native_client_session::tests::logged_client,
        native_entity_refs::tests::{run_with_game_stack, with_client_game},
    };
    use mod_api::{
        __private::game::RegistrationContextNew, BlockState, CustomParticle, ModEntry,
        ParticleProvider, ParticleSpawn, ParticleTickView, RegisterParticleProvidersEvent,
        Registrar, RegistrationContext,
    };
    use std::sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    use sunrise_assets::resource_source::{Resource, ResourceSource};
    use sunrise_client::particles::ExplosionParticle;
    use sunrise_core::random::{LegacyRandom, RandomSource};
    use sunrise_gameplay_common::game::{
        api_values::TryIntoApi, mods::resolve_particle_providers,
        native_hooks::native_register_particle_providers,
    };
    use sunrise_level::chunk::Chunk;
    use sunrise_protocol::{
        connection::Pipe,
        entity_data::{MOD_PARTICLE_ID, Particle, ParticleData},
        packets::play::{Clientbound, LevelParticles, Respawn, SpawnInfo},
    };

    const TYPE: &str = "fixture:encoded";
    const BLOCK: [i32; 3] = [5, 100, 4];

    #[derive(Debug, PartialEq)]
    struct ObservedSpawn {
        options: u32,
        position: [f64; 3],
        velocity: [f64; 3],
        block: BlockState,
    }

    #[derive(Clone, Default)]
    struct Provider {
        ty: Arc<OnceLock<mod_api::ParticleType<u32>>>,
        observed: Arc<Mutex<Vec<ObservedSpawn>>>,
        draw: Arc<AtomicBool>,
        binds: Arc<AtomicUsize>,
    }

    struct Instance;
    impl CustomParticle for Instance {}

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Provider {
        fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
            let ty = registrar.register_particle_type(
                "encoded",
                false,
                mod_api::ParticleMapCodec::new(
                    |_, _| Ok(0u32),
                    |_, _| Ok(mod_api::Compound::default()),
                ),
                mod_api::ParticleStreamCodec::wire(),
            );
            assert_eq!(self.ty.get_or_init(|| ty).key(), ty.key());
        }

        fn register_particle_providers(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterParticleProvidersEvent<'_>,
        ) {
            let provider = self.clone();
            event
                .providers
                .register_sprite_set(*self.ty.get().unwrap(), move |_| {
                    provider.binds.fetch_add(1, Ordering::Relaxed);
                    provider.clone()
                });
        }
    }

    impl ParticleProvider<u32> for Provider {
        fn create_particle(
            &self,
            spawn: &mut ParticleSpawn<'_, u32>,
            _: &mut dyn ParticleTickView,
        ) -> Option<Box<dyn CustomParticle>> {
            if self.draw.load(Ordering::Relaxed) {
                let _ = spawn.random.next_long();
            }
            self.observed.lock().unwrap().push(ObservedSpawn {
                options: *spawn.options,
                position: spawn.position,
                velocity: spawn.velocity,
                block: spawn.level.block_state(BLOCK),
            });
            Some(Box::new(Instance))
        }
    }

    fn sprite_description() -> ResourceSource {
        ResourceSource::indexed(
            [(
                "assets/fixture/particles/encoded.json".into(),
                Resource::bytes(br#"{"textures":["fixture:encoded"]}"#.to_vec()),
            )]
            .into(),
        )
    }

    fn fixture_engine(source: &ResourceSource) -> ParticleEngine {
        let mut factories = mod_api::ParticleProviderRegistry::default();
        native_register_particle_providers(&RegistrationContext::new(), &mut factories);
        let mut engine = ParticleEngine::with_test_providers(
            source,
            &HashMap::from([(
                "fixture:particle/encoded".into(),
                Rect {
                    u0: 0.0,
                    v0: 0.0,
                    u1: 1.0,
                    v1: 1.0,
                    w: 16,
                    h: 16,
                },
            )]),
            factories,
        );
        engine.set_block_state_source(crate::game::particle_block_states);
        engine
    }

    #[test]
    #[ignore = "daily"]
    fn registered_provider_receives_host_and_network_spawns_with_live_level() {
        run_with_game_stack(|| {
            mod_platform::testing::with_mods(Default::default(), || {
                with_client_game(|game| {
                    let (pipe, _server) = Pipe::pair();
                    game.connect_local(pipe, "ParticleFixture").unwrap();
                    let registry = game.pack.assets.registry;
                    let stone = registry.default_state("minecraft:stone").unwrap();
                    let dirt = registry.default_state("minecraft:dirt").unwrap();
                    let stone_api = stone.try_into_api().unwrap();
                    let dirt_api = dirt.try_into_api().unwrap();
                    let [x, y, z] = BLOCK;
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                    game.world.set(x, y, z, stone).unwrap();

                    let provider = Provider::default();
                    let mods = vec![ModEntry::new("fixture", Box::new(provider.clone()))];
                    mod_platform::testing::with_mods(mods.into(), || {
                        // Resolve this fixture's particle options from its own Parts epoch and sprite declaration, keeping the loaded game's block state index.
                        mod_platform::__private::part_tests::install_parts_fixture(
                            mod_api::__private::mods(),
                        )
                        .unwrap();
                        let source = sprite_description();
                        resolve_particle_providers(&source).unwrap();
                        let ty = *provider.ty.get().unwrap();
                        game.client.particles.0 = fixture_engine(&source);

                        let at = [5.5, 101.0, 4.5];
                        let velocity = [f64::from(0xff12ab), 7.0, -2.0];
                        let options = ty.options(257);
                        game.player = Some(
                            PlayerMotion::new(game.motion, Vec3::new(at[0], at[1], at[2])).unwrap(),
                        );
                        game.client.particles.0.camera = at;
                        let host: &mut dyn ClientHost = game;
                        assert!(host.add_particle(&options, at, velocity));

                        // The next construction must see this change through the same level loan path, even though the options arrived via a server packet this time.
                        game.world.set(x, y, z, dirt).unwrap();
                        let packet = LevelParticles {
                            particle: Particle {
                                id: MOD_PARTICLE_ID,
                                data: Some(ParticleData::Mod {
                                    name: TYPE.into(),
                                    options: 513u32.to_le_bytes().to_vec(),
                                }),
                            },
                            override_limiter: false,
                            always_show: false,
                            pos: [6.5, 101.25, 4.5],
                            dist: [0.1, -0.2, 0.3],
                            max_speed: [0.1, 0.1, -0.2],
                            count: 0,
                            randomization: 0,
                        };
                        game.remote_level_particles(&packet);

                        let far = [64.0, 101.0, 4.5];
                        let host: &mut dyn ClientHost = game;
                        assert!(host.add_particle(&options, far, velocity));
                        game.remote_level_particles(&LevelParticles {
                            pos: far,
                            ..packet.clone()
                        });
                        game.client.particles.0.particle_level = 2;
                        let host: &mut dyn ClientHost = game;
                        assert!(host.add_particle(&options, at, velocity));
                        game.remote_level_particles(&packet);

                        // ClientPacketListener.handleParticleEvent multiplies the float fields before widening. These double bits differ from products in f64.
                        let packet_velocity_bits = [
                            0x3f84_7ae1_6000_0000,
                            0xbf94_7ae1_6000_0000,
                            0xbfae_b852_0000_0000,
                        ];
                        assert_eq!(
                            provider.observed.lock().unwrap()[1]
                                .velocity
                                .map(f64::to_bits),
                            packet_velocity_bits,
                        );
                        assert_eq!(
                            *provider.observed.lock().unwrap(),
                            [
                                ObservedSpawn {
                                    options: 257,
                                    position: at,
                                    velocity,
                                    block: stone_api
                                },
                                ObservedSpawn {
                                    options: 513,
                                    position: [6.5, 101.25, 4.5],
                                    velocity: packet_velocity_bits.map(f64::from_bits),
                                    block: dirt_api,
                                },
                            ],
                            "only admitted requests construct; local values and decoded stream values remain distinct",
                        );
                        assert!(game.client.particles.0.is_empty());
                        game.particle_fixture_tick();
                        assert_eq!(game.client.particles.0.len(), 2);

                        // Restart the listener's spread stream for the same packet. Drawing from the real engine stream inside create must not move later spawns.
                        provider.observed.lock().unwrap().clear();
                        game.client.particles.0.particle_level = 0;
                        let multi = LevelParticles {
                            count: 3,
                            override_limiter: true,
                            ..packet
                        };
                        let mut receive = |draw| {
                            provider.draw.store(draw, Ordering::Relaxed);
                            game.client.remote.as_mut().unwrap().particle_packet_random =
                                LegacyRandom::new(19);
                            game.client.particles.0.random().set_seed(23);
                            game.remote_level_particles(&multi);
                            std::mem::take(&mut *provider.observed.lock().unwrap())
                        };
                        let without_draws = receive(false);
                        assert_eq!(without_draws.len(), 3);
                        assert_eq!(
                            receive(true),
                            without_draws,
                            "engine draws cannot shift packet spread"
                        );
                    });
                });
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn declared_native_particle_cold_loads_without_a_sprite_description() {
        run_with_game_stack(|| {
            let provider = Provider::default();
            let mods = vec![ModEntry::new("fixture", Box::new(provider.clone()))];
            mod_platform::testing::with_mods(mods.into(), || {
                // with_client_game completes the fresh Parts epoch using the same registry resolver as content_root; the prepared base has no sprite JSON for this ID.
                with_client_game(|game| {
                    assert!(
                        game.pack
                            .source
                            .read("assets/fixture/particles/encoded.json")
                            .is_err()
                    );
                    let key = mod_api::__private::particle_named_key(TYPE).unwrap();
                    assert_eq!(
                        mod_api::__private::particle_kind(key),
                        Some(mod_api::__private::ParticleKind::Custom)
                    );
                    let mut engine = fixture_engine(&game.pack.source);
                    let level = ParticleLevel::new(&game.world, game.res.facts(), game.collision);
                    let options = provider.ty.get().unwrap().options(7);
                    let options = ParticleOptions::typed(options.typed().unwrap().clone()).unwrap();
                    engine.create_with_level(Some(&level), &options, [0.0; 3], [0.0; 3]);
                    engine.tick(&level);
                    assert_eq!(provider.binds.load(Ordering::Relaxed), 0);
                    assert!(provider.observed.lock().unwrap().is_empty());
                    assert!(engine.is_empty());
                });
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn changing_dimension_discards_queued_typed_explosion_debris() {
        run_with_game_stack(|| {
            let provider = Provider::default();
            let mods = vec![ModEntry::new("fixture", Box::new(provider.clone()))];
            mod_platform::testing::with_mods(mods.into(), || {
                with_client_game(|game| {
                    let _server = logged_client(game);
                    game.client.particles.0 = fixture_engine(&sprite_description());
                    let at = [0.0, 100.0, 0.0];
                    let options = provider.ty.get().unwrap().options(7);
                    let debris = [ExplosionParticle {
                        options: ParticleOptions::typed(options.typed().unwrap().clone()).unwrap(),
                        scaling: 1.0,
                        speed: 1.0,
                        weight: 1,
                    }];
                    game.client.particles.0.camera = at;
                    game.client.particles.0.track_explosion(at, 1.0, 1, &debris);
                    game.particle_fixture_tick();
                    assert_eq!(provider.observed.lock().unwrap().len(), 1);
                    provider.observed.lock().unwrap().clear();

                    game.client.particles.0.track_explosion(at, 1.0, 1, &debris);
                    game.handle_packet(Clientbound::Respawn(Respawn {
                        spawn: SpawnInfo {
                            dimension_type: 0,
                            dimension: "minecraft:the_nether".into(),
                            seed: 0,
                            game_type: 0,
                            previous_game_type: None,
                            debug: false,
                            flat: false,
                            last_death: None,
                            portal_cooldown: 0,
                            sea_level: 32,
                        },
                        keep: 0,
                    }))
                    .unwrap();
                    assert_eq!(game.dim.id(), "minecraft:the_nether");
                    game.client.particles.0.camera = at;
                    game.client.particles.0.particle_level = 0;
                    game.particle_fixture_tick();
                    assert!(provider.observed.lock().unwrap().is_empty());
                    assert!(game.client.particles.0.is_empty());
                });
            });
        });
    }

    #[cfg(any())]
    #[test]
    #[ignore = "daily"]
    fn opening_an_integrated_world_does_not_bind_client_particle_providers() {
        let test = "game::native_client_particles_fixture::tests::opening_an_integrated_world_does_not_bind_client_particle_providers";
        if run_fresh_cases(test, "SUNRISE_PARTICLE_FACTORY_CHILD", &["integrated"]) {
            return;
        }
        run_with_game_stack(|| {
            struct BindProbe(Arc<Mutex<Vec<thread::ThreadId>>>, Provider);
            #[mod_api::__subscriptions]
            impl mod_api::Hooks for BindProbe {
                fn register(&self, registrar: &mut dyn Registrar, context: &RegistrationContext) {
                    mod_api::Hooks::register(&self.1, registrar, context);
                }

                fn register_particle_providers(
                    &self,
                    _: &RegistrationContext,
                    event: &mut RegisterParticleProvidersEvent<'_>,
                ) {
                    let binds = self.0.clone();
                    event
                        .providers
                        .register_sprite_set(*self.1.ty.get().unwrap(), move |_| {
                            binds.lock().unwrap().push(thread::current().id());
                            Provider::default()
                        });
                }
            }

            Game::initialize_mod_runtime_for_tests();
            let binds = Arc::new(Mutex::new(Vec::new()));
            let mods = vec![ModEntry::new(
                "fixture",
                Box::new(BindProbe(binds.clone(), Provider::default())),
            )];
            mod_platform::testing::with_mods(mods.into(), || {
                mod_platform::__private::part_tests::install_parts_fixture(
                    mod_api::__private::mods(),
                )
                .unwrap();
                register_particles(ExecutionRole::Client).unwrap();
                with_client_game(|game| {
                    // Both engines see the description. Only the window may run its factory.
                    game.pack.source = game.pack.source.extended(
                        [(
                            "assets/fixture/particles/encoded.json".into(),
                            Resource::bytes(br#"{"textures":["smoke_0"]}"#.to_vec()),
                        )]
                        .into(),
                    );
                    game.particles.reload(
                        &game.pack.source,
                        &game.gui.atlas.sprites,
                        &game.gui.atlas.translucent,
                    );
                    let client_thread = thread::current().id();
                    assert_eq!(*binds.lock().unwrap(), [client_thread]);

                    game.settings.render_distance = 2;
                    game.settings.simulation_distance = 2;
                    let options = sunrise_client::title::WorldOptions {
                        world_type: sunrise_client::save::WorldPreset::Flat,
                        ..Default::default()
                    };
                    game.open_integrated("particle-factories", None, Some(1), true, Some(&options))
                        .unwrap();
                    game.integrated
                        .take()
                        .expect("integrated server started")
                        .stop()
                        .unwrap();
                    assert_eq!(*binds.lock().unwrap(), [client_thread]);
                });
            });
        });
    }
}
