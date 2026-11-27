//! Brushing suspicious sand and gravel: `BrushItem.onUseTick` and
//! `BrushableBlockEntity`. The block's loot table stays in its saved block
//! entity (like a decorated pot's) until the first counted stroke rolls it.

#[allow(unused_imports)]
pub use sunrise_gameplay_common::brushing::{Brushing, USE_DURATION, completion_state};

pub trait BrushGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    fn set_dusted(&mut self, pos: [i32; 3], dusted: i32) {
        let registry = self.pack.assets.registry;
        let Some(mut state) = registry
            .state(self.world.block(pos[0], pos[1], pos[2]))
            .cloned()
        else {
            return;
        };
        state.properties.insert("dusted".into(), dusted.to_string());
        if let Ok(state) = registry.resolve(&state) {
            let _ = self.level_set(pos, state);
        }
    }

    fn schedule_brushable_tick(&mut self, pos: [i32; 3]) {
        let at = sunrise_core::position::BlockPos::new(pos[0], pos[1], pos[2]);
        self.with_level(|b, level, ticks| b.schedule_tick(level, ticks, at, 2));
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> BrushGameExt1<'__sg> for G {}

use crate::game::GameGameExt1 as _;
