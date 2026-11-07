//! Client glue for sapling growth and bone meal: installs the live feature
//! registry, routes `BoneMealItem.useOn` through the behaviour library, and
//! crops a growing plant's head with shears (`ShearsItem.useOn`).
use super::{Game, native_hooks::BonemealAsk};
use horizon_client::{Result, items::ItemStack, vanillagen};
use horizon_core::{geometry::Direction, position::BlockPos};
use horizon_world::{level::classes::Class, worldgen::live::LiveFeatures};
use std::sync::Arc;

impl Game<'_> {
    /// Load the configured/placed feature registry used by saplings, fungi
    /// and bone meal (once per process; shares the generator's bundle).
    pub(super) fn install_live_features(&self) {
        install_live_features(&self.res, &self.pack, &self.options);
    }
    /// `BonemealEvent` for the acting hand's bone meal `stack` on `pos`:
    /// the event's answer, and the stack as the mods left it, which goes
    /// back in the hand (`useItemOn` puts a creative player's count back).
    pub(super) fn held_bonemeal(
        &mut self,
        pos: BlockPos,
        stack: &ItemStack,
    ) -> (BonemealAsk, ItemStack) {
        let mut held = stack.clone();
        let ask = self.bonemeal(pos, true, &mut held);
        if held != *stack {
            if self.creative {
                held.count = stack.count;
            }
            *self.held_mut() = (held.count > 0).then(|| held.clone());
        }
        (ask, held)
    }
    /// `BoneMealItem.useOn` for the targeted block.
    pub(super) fn use_bone_meal(&mut self, stack: &ItemStack) -> Result<bool> {
        if &*stack.item != "minecraft:bone_meal" {
            return Ok(false);
        }
        let Some(hit) = self.pick() else {
            return Ok(false);
        };
        let face = match hit.face {
            [0, 1, 0] => Direction::Up,
            [0, -1, 0] => Direction::Down,
            [0, 0, -1] => Direction::North,
            [0, 0, 1] => Direction::South,
            [-1, 0, 0] => Direction::West,
            _ => Direction::East,
        };
        let pos = BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]);
        let (ask, held) = self.held_bonemeal(pos, stack);
        let used = self.with_level(|b, level, ticks| {
            b.use_bone_meal(level, ticks, pos, face, ask.crop, ask.check)
        });
        // A canceled `BonemealEvent` that succeeded uses none of the meal.
        let consumed = used && ask.crop != Some(true);
        // ItemStack.causeUseVibration at the player, after the meal is used
        // up (shrunk even in creative): an emptied stack has no use effects.
        if used
            && held.count > u32::from(consumed)
            && let Some(p) = self.player.as_ref().map(|p| p.position)
        {
            let ctx = self.player_context(None);
            self.post_game_event("minecraft:item_interact_finish", [p.x, p.y, p.z], ctx);
        }
        self.process_level_events()?;
        if used {
            self.swing_arm();
            // ItemStack.useOn: an item interaction counts as a use.
            self.award_stat("used", &stack.item, 1);
            if consumed {
                self.consume_held();
            }
        }
        Ok(used)
    }
    /// `ShearsItem.useOn` on a growing plant's head below its top age (kelp,
    /// weeping and twisting vines, cave vines): the head goes to
    /// `GrowingPlantHeadBlock.getMaxAgeState` (age 25) with
    /// `block.growing_plant.crop`, the level hears a `block_change`, and the
    /// shears wear by one. True when it cropped (SUCCESS).
    pub(super) fn shears_crop(&mut self, pos: [i32; 3], stack: &ItemStack) -> Result<bool> {
        if &*stack.item != "minecraft:shears" {
            return Ok(false);
        }
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let b = self.res.behaviours();
        // A head grows and carries `AGE_25`; the plant's body below it does not.
        if !b.data(state).is(Class::GrowingPlantHead) {
            return Ok(false);
        }
        // `GrowingPlantHeadBlock.isMaxAge`: the property's top value.
        if b.table.int(state, b.props.age) == 25 {
            return Ok(false);
        }
        // `GrowingPlantHeadBlock.getMaxAgeState`.
        let max_age = b.table.set_int(state, b.props.age, 25);
        self.audio.play(
            "minecraft:block.growing_plant.crop",
            Some([
                f64::from(pos[0]) + 0.5,
                f64::from(pos[1]) + 0.5,
                f64::from(pos[2]) + 0.5,
            ]),
            1.0,
            1.0,
        );
        self.level_set(pos, max_age)?;
        self.post_block_game_event(
            "minecraft:block_change",
            pos,
            self.sculk_actor(),
            Some(max_age),
        );
        self.damage_held(1);
        // ItemStack.useOn: an item interaction counts as a use.
        self.award_stat("used", &stack.item, 1);
        self.swing_arm();
        // ServerPlayerGameMode.useItemOn: a consuming useOn reports it.
        self.trigger_block("minecraft:item_used_on_block", pos, stack);
        Ok(true)
    }
}

/// Shared by the initialization worker and fixture setup.
pub(super) fn install_live_features(
    res: &super::Resources,
    pack: &super::PackResources,
    options: &super::Options,
) {
    if res.behaviours().live_features().is_some() {
        return;
    }
    let dir = options.resources.join("worldgen");
    let started = std::time::Instant::now();
    let features = vanillagen::shared_data_from(
        &dir,
        &Default::default(),
        pack.assets.registry,
        &pack.data_changes,
    )
    .map_err(|e| e.to_string())
    .and_then(|data| LiveFeatures::new(&data).map_err(|e| e.to_string()));
    match features {
        Ok(Some(features)) => {
            res.behaviours().set_live_features(Arc::new(features));
            log::info!("Horizon: live features ready in {:?}", started.elapsed());
        }
        Ok(None) => {
            log::warn!("Horizon: the worldgen bundle has no features; saplings will not grow")
        }
        Err(e) => log::warn!("Horizon: live features unavailable: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use horizon_client::items::ItemStack;
    use horizon_core::{geometry::Direction, position::BlockPos};
    use mod_api::{self, BonemealEvent, BonemealSource, Flow, Hooks, HostSide};
    use std::sync::{Mutex, MutexGuard, PoisonError};

    /// What each `bonemeal` listener heard: the player, the target check
    /// and the source. The test that reads it runs in a process of its own,
    /// so no other test's events cross into it; a failed assertion doesn't
    /// hide the next one behind poison.
    static HEARD: Mutex<Vec<(Option<u128>, bool, BonemealSource)>> = Mutex::new(Vec::new());

    /// Sets the stack's count to `count`, then cancels with `successful`.
    struct Meal {
        count: Option<u32>,
        cancel: Option<bool>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Meal {
        fn bonemeal(&self, _: HostSide, event: &mut BonemealEvent<'_>) -> Flow {
            heard().push((event.player, event.valid_bonemeal_target, event.source));
            if let Some(count) = self.count {
                if count == 0 {
                    *event.stack = None;
                } else if let Some(stack) = event.stack.as_mut() {
                    stack.set_count(count).unwrap();
                }
            }
            let Some(successful) = self.cancel else {
                return Flow::Continue;
            };
            event.successful = successful;
            Flow::Cancel
        }
    }

    fn heard() -> MutexGuard<'static, Vec<(Option<u128>, bool, BonemealSource)>> {
        HEARD.lock().unwrap_or_else(PoisonError::into_inner)
    }

    const CROP: BlockPos = BlockPos { x: 0, y: 64, z: 0 };

    /// A dispenser at (0, 64, 1) facing young wheat, with `meal` listening:
    /// what the slot holds after one dispense, and the wheat's age.
    fn dispense(meal: Meal, target: &str) -> (Option<ItemStack>, i32) {
        heard().clear();
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("meal", Box::new(meal))].into();
        let mut out = None;
        mod_api::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                game.world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
                );
                let registry = game.pack.assets.registry;
                let farmland = registry.default_state("minecraft:farmland").unwrap();
                game.world.set(0, 63, 0, farmland).unwrap();
                let block = registry.default_state(target).unwrap();
                game.world.set(CROP.x, CROP.y, CROP.z, block).unwrap();
                let left = game
                    .dispense_item(
                        (0, 64, 1),
                        Direction::North,
                        ItemStack::new("minecraft:bone_meal", 3),
                    )
                    .unwrap();
                let b = game.res.behaviours();
                let state = game.world.block(CROP.x, CROP.y, CROP.z);
                out = Some((left, b.table.int(state, b.props.age)));
            });
        });
        out.unwrap()
    }

    /// A successful cancel keeps the meal and grows nothing; an unsuccessful
    /// one fails; an untouched use grows the stack the mod edited. One test,
    /// run in order in a fresh process, since every case reads `HEARD`.
    #[test]
    #[ignore = "needs prepared resources and an isolated process for native mods"]
    fn a_dispensers_bonemeal_follows_the_cancel_and_the_edited_stack() {
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "game::growth::tests::a_dispensers_bonemeal_follows_the_cancel_and_the_edited_stack",
            "HORIZON_BONEMEAL_DISPENSE_CHILD",
            &["dispense"],
        ) {
            return;
        }
        let (left, age) = dispense(
            Meal {
                count: None,
                cancel: Some(true),
            },
            "minecraft:wheat",
        );
        assert_eq!(left.map(|s| s.count), Some(3));
        assert_eq!(age, 0);
        // A dispenser has no player; the young wheat is a target.
        assert_eq!(*heard(), [(None, true, BonemealSource::Interaction)]);

        let (left, age) = dispense(
            Meal {
                count: None,
                cancel: Some(false),
            },
            "minecraft:wheat",
        );
        assert_eq!((left.map(|s| s.count), age), (Some(3), 0));
        // Left to vanilla, growCrop uses one of the stack the mod edited.
        let (left, age) = dispense(
            Meal {
                count: Some(5),
                cancel: None,
            },
            "minecraft:wheat",
        );
        assert_eq!(left.map(|s| s.count), Some(4));
        assert!(age > 0, "the wheat grew");
        // Stone is no target, and the dispense fails with the meal kept.
        let (left, _) = dispense(
            Meal {
                count: None,
                cancel: None,
            },
            "minecraft:stone",
        );
        assert_eq!(left.map(|s| s.count), Some(3));
        assert_eq!(*heard(), [(None, false, BonemealSource::Interaction)]);
    }

    #[test]
    #[ignore = "needs prepared resources and an isolated process for native mods"]
    fn the_hand_keeps_the_mods_stack_and_a_creative_count() {
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "meal",
            Box::new(Meal {
                count: Some(1),
                cancel: Some(true),
            }),
        )]
        .into();
        mod_api::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                let stack = ItemStack::new("minecraft:bone_meal", 3);
                *game.held_mut() = Some(stack.clone());
                assert_eq!(game.held_bonemeal(CROP, &stack).0.crop, Some(true));
                assert_eq!(game.held().map(|s| s.count), Some(1));
                game.creative = true;
                *game.held_mut() = Some(stack.clone());
                let (ask, held) = game.held_bonemeal(CROP, &stack);
                assert_eq!((ask.crop, held.count), (Some(true), 3));
                assert_eq!(game.held().map(|s| s.count), Some(3));
            });
        });
    }
}
