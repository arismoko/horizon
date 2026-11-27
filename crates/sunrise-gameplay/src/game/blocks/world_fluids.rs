//! A mod's fluid container picking up and placing fluid in the world, as NeoForge's
//! `FluidUtil.tryPickupFluid` and `tryPlaceFluid` do through a bucket's own
//! `BucketPickup.pickupBlock` and `BucketItem.emptyContents`. Everything is decided at the call,
//! against the world as the transaction has it; the block is written then without its updates
//! and kept by the journal, and the rest is done at the outermost commit. An abort leaves the
//! world as it was, which the reference doesn't promise.
use mod_api::{
    FluidKey, FluidStack, GameEventContext, LevelRef, PlayerRef, SoundSource, TransactionContext,
};
use sunrise_core::position::BlockPos;
use sunrise_gameplay_common::game::transactions::After;
use sunrise_gameplay_common::game::{
    spill_common::Spill,
    world_fluid_data::{FluidEffects, WrittenFluid},
};
use sunrise_world::{
    fluid::FluidKind,
    level::{
        BucketContent, FluidChange, FluidWrite, LiquidUser, fluid_empty_sound, fluid_pickup_sound,
    },
};

/// `FluidType.BUCKET_VOLUME`: what one source block holds.
const BUCKET: u32 = 1000;

impl crate::game::ServerGame<'_> {
    /// A normal player's bucket uses the same plan as a dispenser, but lends a
    /// transactional host to a subscribing container instead of the direct one.
    pub fn empty_player_bucket(
        &mut self,
        pos: BlockPos,
        beside: Option<BlockPos>,
        content: BucketContent,
        sound: &str,
        sneaking: bool,
    ) -> Option<BlockPos> {
        let user = self.bucket_liquid_user();
        let Some(kind) = content
            .fluid()
            .filter(|_| mod_api::__private::has_scoped_hook(mod_api::HookId::PlaceLiquid))
        else {
            return self.with_level(|b, level, ticks| {
                b.bucket_empty_as(level, ticks, pos, beside, content, sound, sneaking, user)
            });
        };
        let mut write = self.with_level(|b, level, ticks| {
            b.plan_bucket_pour(level, ticks, pos, beside, kind, sneaking, user)
        })?;
        if matches!(write.change, FluidChange::Fill(_))
            && let Some(level) = self.level(self.common.dim.id())
            && self.journalled_place_liquid(level, &write).is_some()
        {
            // BucketItem still empties on a container's false answer.
            write.change = FluidChange::Fill(None);
        }
        self.with_level(|b, level, ticks| {
            b.apply_bucket_pour(
                level,
                ticks,
                write,
                sound,
                content == BucketContent::MobWater,
            )
        })
    }
    /// `BucketPickup.pickupBlock` for a mod's fluid container at `pos` in the level in place,
    /// kept by transaction `tx`: the source taken, a bucket of it. `Ok(None)`, with nothing done,
    /// if the block gives no fluid or the level is not in place; `Err(TransactionError::InvalidContext)`
    /// if `tx` is not current.
    pub(in crate::game) fn level_pickup_fluid(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        player: Option<PlayerRef>,
        tx: &TransactionContext,
    ) -> Result<Option<FluidStack>, mod_api::TransactionError<()>> {
        if !self.transaction_context_is_current(tx) {
            return Err(mod_api::TransactionError::InvalidContext);
        }
        if !self.fluid_write_allowed(level, pos) {
            return Ok(None);
        }
        let creative = self.creative(player);
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        let Some(write) =
            self.with_level(|b, level, ticks| b.plan_fluid_pickup(level, ticks, at, creative))
        else {
            return Ok(None);
        };
        let Some(key) = mod_api::__private::fluid_key(write.fluid.name()) else {
            return Ok(None);
        };
        self.journal_fluid_write(level, write, true, player);
        Ok(Some(FluidStack::new(key, BUCKET)))
    }
    /// `BucketItem.emptyContents` of a bucket of `fluid` at `pos` in the level in place, for
    /// a mod's fluid container, kept by transaction `tx`. Whether it was placed, or hissed away
    /// where water evaporates. `Ok(false)`, with nothing done, for a fluid without a world form,
    /// where it can't be placed, or if the level is not in place; `Err(TransactionError::InvalidContext)`
    /// if `tx` is not current.
    pub fn level_place_fluid(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        fluid: FluidKey,
        player: Option<PlayerRef>,
        validate: bool,
        tx: &TransactionContext,
    ) -> Result<bool, mod_api::TransactionError<()>> {
        if !self.transaction_context_is_current(tx) {
            return Err(mod_api::TransactionError::InvalidContext);
        }
        if !self.fluid_write_allowed(level, pos) {
            return Ok(false);
        }
        let Some(kind) = mod_api::__private::fluid_id(fluid)
            .and_then(FluidKind::from_name)
            .filter(|kind| *kind != FluidKind::Empty)
        else {
            return Ok(false);
        };
        let user = player
            .and_then(|player| {
                Some(LiquidUser {
                    entity_id: Some(player.entity().id()),
                    creative: self.player_is_creative(player)?,
                })
            })
            .unwrap_or_default();
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        let Some(mut write) = self.with_level(|b, level, ticks| {
            b.plan_bucket_pour(level, ticks, at, None, kind, false, user)
        }) else {
            return Ok(false);
        };
        if let FluidChange::Fill(template) = write.change {
            let answer = self.journalled_place_liquid(level, &write);
            if validate && !answer.unwrap_or(template.is_some()) {
                return Ok(false);
            }
            if answer.is_some() {
                write.change = FluidChange::Fill(None);
            }
        }
        self.journal_fluid_write(level, write, false, player);
        Ok(true)
    }
    fn fluid_write_allowed(&self, level: LevelRef, pos: [i32; 3]) -> bool {
        self.current_transaction_is_here()
            && self.level_in_place(level)
            && self.native_level_write_allowed(level, pos)
    }
    fn creative(&self, player: Option<PlayerRef>) -> bool {
        player.is_some_and(|player| self.player_is_creative(player) == Some(true))
    }
    /// What `write` does at the call. The block entity a new block of another kind removes is
    /// kept whole by the journal before any mod code runs; then the broken block's drops are
    /// made from the block as it is, with its block entity still there (the `block_drops`
    /// listeners, which only read, see the old block, and whatever unpacking its loot changes
    /// in the entity an abort puts back; a cancelled drop keeps the replacement); then the new
    /// block is stored, without its updates, and that block entity goes. The rest waits for
    /// the commit.
    fn journal_fluid_write(
        &mut self,
        level: LevelRef,
        write: FluidWrite,
        pickup: bool,
        player: Option<PlayerRef>,
    ) {
        let pos = [write.pos.x, write.pos.y, write.pos.z];
        let key = (pos[0], pos[1], pos[2]);
        let b = self.common.res.behaviours();
        let to = write.to(b);
        let removes = to != write.from
            && b.block(to) != b.block(write.from)
            && self.common.world.block_entities.get(&key).is_some();
        if removes {
            self.journal_block_entity(key);
        }
        let drops = match write.broken() {
            Some(broken) => self.with_level(|b, level, ticks| {
                b.drop_resources(level, ticks, write.pos, broken);
                std::mem::take(level.spill)
            }),
            None => Spill::default(),
        };
        if to != write.from {
            self.journal_set_block(pos, to);
        }
        if removes {
            self.journal_remove_block_entity(key);
        }
        self.after_commit(After::FluidWrite(Box::new(WrittenFluid {
            level,
            write,
            drops,
            pickup,
            player,
        })));
    }
    /// `FluidUtil.triggerSoundAndGameEvent` for `fluid` at the centre of `pos` in the level in
    /// place, at the outermost commit of transaction `tx`.
    pub(in crate::game) fn level_play_fluid_effects(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        fluid: FluidKey,
        pickup: bool,
        player: Option<PlayerRef>,
        tx: &TransactionContext,
    ) -> Result<bool, mod_api::TransactionError<()>> {
        if !self.transaction_context_is_current(tx) {
            return Err(mod_api::TransactionError::InvalidContext);
        }
        if !self.current_transaction_is_here() || !self.level_in_place(level) {
            return Ok(false);
        }
        if mod_api::__private::fluid_id(fluid).is_none() {
            return Ok(false);
        }
        let tables =
            sunrise_gameplay_common::game::native_block_tables::BlockTables::of(&self.common);
        let sound = match pickup {
            true => tables.fluid_pickup_sound(fluid),
            false => tables.fluid_empty_sound(fluid),
        }
        .ok()
        .flatten();
        self.after_commit(After::FluidEffects(Box::new(FluidEffects {
            level,
            pos,
            // A tank-only fluid that names a bucket has its sounds too.
            sound,
            pickup,
            player,
        })));
        Ok(true)
    }
    /// The rest of a world pickup or placement at the outermost commit, its block's updates
    /// done already: the fluid tick, the broken block's drops, its particles and
    /// `block_destroy`, a doused block's or a dried ghast's sound, then the bucket's sound and
    /// fluid game event (the hiss alone where water evaporates).
    pub(in crate::game) fn commit_fluid_write(&mut self, written: WrittenFluid) {
        let WrittenFluid {
            level,
            write,
            drops,
            pickup,
            player,
        } = written;
        self.with_level(|b, level, ticks| b.fluid_write_ticks(level, ticks, &write));
        debug_assert!(
            self.server.spill.0.is_empty(),
            "a spill left between level calls"
        );
        self.server.spill.0 = drops;
        self.flush_spill();
        self.with_level(|b, level, ticks| b.fluid_write_effects(level, ticks, &write));
        let _ = self.server_process_level_events();
        if write.sounds() {
            let pos = [write.pos.x, write.pos.y, write.pos.z];
            self.commit_fluid_effects(FluidEffects {
                level,
                pos,
                sound: match pickup {
                    true => fluid_pickup_sound(write.fluid),
                    false => fluid_empty_sound(write.fluid),
                },
                pickup,
                player,
            });
        }
    }
    /// The bucket's sound (none for a fluid with no world form) and fluid game event, from the
    /// player when one is given, at the outermost commit.
    pub(in crate::game) fn commit_fluid_effects(&mut self, effects: FluidEffects) {
        let FluidEffects {
            level,
            pos,
            sound,
            pickup,
            player,
        } = effects;
        let at = pos.map(|c| f64::from(c) + 0.5);
        let event = match pickup {
            true => "minecraft:fluid_pickup",
            false => "minecraft:fluid_place",
        };
        if let Some(sound) = sound {
            self.level_play_sound(level, sound, at, SoundSource::Blocks, 1.0, 1.0, None);
        }
        let context = GameEventContext {
            source_entity: player.map(PlayerRef::entity),
            affected_state: None,
        };
        self.level_game_event(level, event, at, context);
    }
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_level_refs_common::NativeLevelRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
