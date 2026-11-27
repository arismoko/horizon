#[cfg(any())]
pub mod own_block_entity_tests {

    use super::{BlockEntityTick, BlockEntityTickSlot, OwnBlockEntities, TickedKind};

    use crate::game::published_loops::LoopFlow;

    use std::sync::{Arc, OnceLock};

    use sunrise_level::block_entities::Pos;

    use sunrise_level::containers::{BlockEntity, ModBlockEntity};

    pub const A: &str = "minecraft:p3_machine";
    pub const B: &str = "own_b:machine";
    pub const DETECTOR: &str = "minecraft:daylight_detector";
    #[cfg(test)]
    impl Machines {
        fn new(block: &'static str, also_on: Option<&'static str>) -> Self {
            Self {
                block,
                also_on,
                keys: OnceLock::new(),
            }
        }
    }
    #[cfg(test)]
    #[mod_api::__subscriptions]
    impl Hooks for Machines {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let kind = r
                .block_entity_type(self.block)
                .blocks(&[self.block])
                .build();
            r.block_entity_part(&kind, Ticks(self.block)).unwrap();
            if let Some(other) = self.also_on {
                let keys = (kind, r.block_key(other));
                assert_eq!(*self.keys.get_or_init(|| keys), keys);
            }
        }
        fn blocks(&self, _: &RegistrationContext, event: &mut BlocksEvent<'_>) {
            event
                .register(mod_api::Block::new(self.block, "minecraft:stone"))
                .unwrap();
        }
        fn block_entity_type_add_blocks(
            &self,
            _: &RegistrationContext,
            event: &mut BlockEntityTypeAddBlocksEvent<'_>,
        ) {
            if let Some(&(kind, block)) = self.keys.get() {
                event.modify(kind, &[block]);
            }
        }
    }
    #[cfg(test)]
    /// Makes block `self.0`'s block entity tick, and notes each tick in the entity's data:
    /// [`mark`] of the type whose part ran.
    struct Ticks(&'static str);
    #[cfg(test)]
    #[mod_api::sunrise_part]
    impl BlockPart for Ticks {}
    #[cfg(test)]
    #[mod_api::sunrise_part]
    impl BlockEntityPart for Ticks {
        fn block_entity_tick(&self, event: &mut BlockEntityTickEvent<'_>) {
            event.subject.with_view(|mut entity| {
                let mut data = entity.data_slots().to_vec();
                data.push(mark(self.0));
                assert!(entity.set_data_slots(&data));
            });
        }
    }
    /// What a tick of mod A's type (1) or mod B's (2) leaves in a block entity's data.
    pub fn mark(kind: &str) -> i32 {
        if kind == A { 1 } else { 2 }
    }
    #[cfg(test)]
    impl Count {
        fn new(ids: &'static [&'static str]) -> Self {
            Self {
                own: OwnBlockEntities::new(ids),
                visited: 0,
                mine: Vec::new(),
            }
        }
    }
    #[cfg(test)]
    impl BlockEntityTickSlot for Count {
        fn before(&mut self, tick: &mut BlockEntityTick<'_, '_>) -> LoopFlow {
            self.visited += 1;
            if self.own.contains(tick) {
                self.mine.push(tick.pos);
            }
            LoopFlow::Continue
        }
    }
    #[cfg(test)]
    impl BlockEntityTickSlot for Swap {
        fn before(&mut self, tick: &mut BlockEntityTick<'_, '_>) -> LoopFlow {
            if self.at == Some(tick.pos) {
                self.at = None;
                let Some(BlockEntity::Mod(entity)) = tick.block_entity() else {
                    panic!("no mod block entity at {:?}", tick.pos);
                };
                **entity = ModBlockEntity::new(B.into(), Default::default());
            }
            LoopFlow::Continue
        }
        fn after(&mut self, tick: &mut BlockEntityTick<'_, '_>) {
            self.after.push(tick.pos);
        }
    }
    #[cfg(test)]
    /// For each entry: its position, whether the cursor is of mod A's type and of mod B's, and
    /// the type of the block entity standing there.
    type Entry = (Pos, bool, bool, &'static str);
    #[cfg(test)]
    impl Seen {
        fn new() -> Self {
            Self {
                a: OwnBlockEntities::new(&[A]),
                b: OwnBlockEntities::new(&[B]),
                before: Vec::new(),
                after: Vec::new(),
            }
        }
        fn entry(&mut self, tick: &mut BlockEntityTick<'_, '_>) -> Entry {
            assert_eq!(tick.kind, TickedKind::Mod);
            let standing = match tick.block_entity() {
                Some(BlockEntity::Mod(entity)) if entity.id() == A => A,
                Some(BlockEntity::Mod(entity)) if entity.id() == B => B,
                _ => "other",
            };
            (
                tick.pos,
                self.a.contains(tick),
                self.b.contains(tick),
                standing,
            )
        }
    }
    #[cfg(test)]
    impl BlockEntityTickSlot for Seen {
        fn before(&mut self, tick: &mut BlockEntityTick<'_, '_>) -> LoopFlow {
            let entry = self.entry(tick);
            self.before.push(entry);
            LoopFlow::Continue
        }
        fn after(&mut self, tick: &mut BlockEntityTick<'_, '_>) {
            let entry = self.entry(tick);
            self.after.push(entry);
        }
    }
}
