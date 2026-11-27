use crate::game::ServerGame;
use sunrise_core::{geometry::Direction, position::BlockPos};
use sunrise_game_data::Result;
use sunrise_gameplay_common::game::{
    growth_features::install_live_features,
    item_abilities::{ItemAbilitiesGameExt1 as _, ability::SHEARS_TRIM},
};
use sunrise_items::items::ItemStack;
use sunrise_world::level::classes::Class;

impl ServerGame<'_> {
    /// Load the configured/placed feature registry used by saplings, fungi
    /// and bone meal (for fixtures: the plain bundle, unless one is installed; worlds install
    /// their own in `prepare_world`).
    pub fn install_live_features(&self) {
        if self.common.res.behaviours().live_features().is_some() {
            return;
        }
        install_live_features(
            &self.common.res,
            &self.common.pack,
            &self.common.options,
            &Default::default(),
        );
    }
    /// `ShearsItem.useOn` on a growing plant's head below its top age (kelp,
    /// weeping and twisting vines, cave vines): the head goes to
    /// `GrowingPlantHeadBlock.getMaxAgeState` (age 25) with
    /// `block.growing_plant.crop`, the level hears a `block_change`, and the
    /// shears wear by one. True when it cropped (SUCCESS).
    pub fn shears_crop(&mut self, pos: [i32; 3], stack: &ItemStack) -> Result<bool> {
        if !self.stack_shears(stack, SHEARS_TRIM) {
            return Ok(false);
        }
        let state = self.common.world.block(pos[0], pos[1], pos[2]);
        let b = self.common.res.behaviours();
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
        self.server_play_sound(
            "minecraft:block.growing_plant.crop",
            [
                f64::from(pos[0]) + 0.5,
                f64::from(pos[1]) + 0.5,
                f64::from(pos[2]) + 0.5,
            ],
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
        self.trigger_block("minecraft:item_used_on_block", pos, stack);
        self.damage_held(1);
        // ItemStack.useOn: an item interaction counts as a use.
        self.award_stat("used", &stack.item, 1);
        self.swing_arm();
        Ok(true)
    }
    /// `BoneMealItem.useOn` for the targeted block.
    pub(in crate::game::server_side) fn use_bone_meal(
        &mut self,
        stack: &ItemStack,
    ) -> Result<bool> {
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
        let mut held = stack.clone();
        let ask = self.bonemeal(pos, true, &mut held);
        if held != *stack {
            if self.common.creative {
                held.count = stack.count;
            }
            *self.common.held_mut() = (held.count > 0).then(|| held.clone());
        }
        let used = self.with_level(|b, level, ticks| {
            b.use_bone_meal(level, ticks, pos, face, ask.crop, ask.check)
        });
        // A canceled `BonemealEvent` that succeeded uses none of the meal.
        let consumed = used && ask.crop != Some(true);
        // ItemStack.causeUseVibration at the player, after the meal is used
        // up (shrunk even in creative): an emptied stack has no use effects.
        if used
            && held.count > u32::from(consumed)
            && let Some(p) = self.common.player.as_ref().map(|p| p.position)
        {
            let ctx = self.player_context(None);
            self.post_game_event("minecraft:item_interact_finish", [p.x, p.y, p.z], ctx);
        }
        self.server_process_level_events()?;
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
}

use sunrise_gameplay_common::game::side_ops::presentation::PresentationSideGameExt as _;

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::item_use_stacks::ItemUseStacksGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::ModGameExt1 as _;

use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;
