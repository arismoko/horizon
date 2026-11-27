use crate::game::native_entity_refs::tests::run_fresh_cases;
use mod_api::{
    AddDestroyEffectsEvent, AddHitEffectsEvent, BlockPart, BlockPos, BlockShapeKind, ClientHost,
    Direction, Flow, Hooks, ParticleKey, ParticleOptions, Registrar, RegistrationContext,
};
use std::sync::{Arc, Mutex};

pub(in crate::game) type Calls = Arc<Mutex<Vec<(String, BlockPos, Option<Direction>)>>>;
pub(in crate::game) struct Effects {
    pub(in crate::game) calls: Calls,
    pub(in crate::game) marker: ParticleKey,
}
#[mod_api::sunrise_part]
impl BlockPart for Effects {
    fn add_hit_effects(
        &self,
        host: &mut dyn ClientHost,
        event: &mut AddHitEffectsEvent<'_>,
    ) -> Flow {
        let id = event.state.block().id(&*host).unwrap().to_owned();
        self.calls
            .lock()
            .unwrap()
            .push((id.clone(), event.pos, Some(event.face)));
        if id != "minecraft:glass" {
            return Flow::Continue;
        }
        use std::ops::ControlFlow;
        let mut boxes = Vec::new();
        assert_eq!(
            event.level.visit_block_shape(
                &*host,
                event.state,
                [8, 64, 4],
                BlockShapeKind::Outline,
                &mut |bounds| {
                    boxes.push(bounds);
                    ControlFlow::Continue(())
                }
            ),
            Some(ControlFlow::Continue(()))
        );
        assert_eq!(boxes, [[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]]);
        event.random.next_int();
        assert_eq!(
            event
                .level
                .add_breaking_particles(host, [8, 64, 4], event.state, event.face),
            Ok(true)
        );
        Flow::Cancel
    }

    fn add_destroy_effects(
        &self,
        host: &mut dyn ClientHost,
        event: &mut AddDestroyEffectsEvent<'_>,
    ) -> Flow {
        let id = event.state.block().id(&*host).unwrap().to_owned();
        self.calls
            .lock()
            .unwrap()
            .push((id.clone(), event.pos, None));
        if id == "minecraft:glass" {
            // The sound capture records how many of these requests have already run.
            assert!(host.add_particle(
                &ParticleOptions::simple(self.marker),
                [4.5, 64.5, 4.5],
                [0.0; 3]
            ));
            assert_eq!(
                event.level.add_terrain_particle(
                    host,
                    event.state,
                    [4, 64, 4],
                    [8.5, 64.5, 4.5],
                    [0.0; 3]
                ),
                Ok(true)
            );
        }
        Flow::Cancel
    }
}

pub(in crate::game) struct Register(pub(in crate::game) Calls);
#[mod_api::__subscriptions]
impl Hooks for Register {
    fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
        let marker = registrar.particle_key("minecraft:heart");
        for name in [
            "minecraft:glass",
            "minecraft:dirt",
            "minecraft:barrier",
            "minecraft:air",
        ] {
            let block = registrar.block_key(name);
            registrar.block_part(
                block,
                Effects {
                    calls: self.0.clone(),
                    marker,
                },
            );
        }
    }
}

pub fn fresh(name: &str) -> bool {
    run_fresh_cases(
        &format!("game::native_hooks::block_particle_effects_tests::{name}"),
        "SUNRISE_BLOCK_PARTICLE_EFFECT_CHILD",
        &["run"],
    )
}
