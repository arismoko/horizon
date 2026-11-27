//! Allays on the game side: spotting and picking up matching drops,
//! throwing their load to the player or note block, dancing to jukeboxes,
//! and the hand-over interaction (`Allay.mobInteract`).

use super::level_sounds_common::LevelSound;
use crate::game::ServerGame;
use mod_api::SoundSource;
use sunrise_gameplay::allays::{AllayLevel, AllayPlayer, SightView};
use {
    sunrise_items::items::ItemStack, sunrise_sim::first_person_sim::Hand,
    sunrise_sim::mobs::Species, sunrise_sim::mobs::allay,
};

impl crate::game::ServerGame<'_> {
    /// `Allay.mobInteract`'s `level().playSound(player, this, sound, NEUTRAL,
    /// 2, 1)`, bound to allay `allay`. Vanilla leaves the player out because
    /// their client played it; Sunrise's own client plays it from the
    /// packet, so every player gets it.
    fn play_allay_sound(&mut self, sound: &str, at: [f64; 3], allay: u32) {
        let sound = LevelSound::new(sound, at, 2.0, 1.0).in_source(SoundSource::Neutral);
        self.play_bound_sound(sound, allay, None);
    }

    pub(in crate::game) fn tick_allays(&mut self) {
        sunrise_gameplay::allays::tick_allays(self);
    }
    /// `Allay.mobInteract`: duplicate when dancing with an amethyst shard,
    /// take the held item, or hand back what it holds.
    pub(in crate::game) fn interact_allay(
        &mut self,
        index: usize,
        item: Option<&ItemStack>,
    ) -> bool {
        let m = &self.common.mobs[index];
        if m.species() != Species::Allay || !m.alive() {
            return false;
        }
        let at = [m.position.x, m.position.y, m.position.z];
        let shard = item.is_some_and(|s| {
            self.items()
                .item_tags
                .get("minecraft:duplicates_allays")
                .is_some_and(|tag| tag.contains(&*s.item))
        });
        if m.extra.flag && shard && m.extra.egg_time == 0 {
            let pos = at;
            self.common.mobs[index].extra.egg_time = 6000;
            if let Some(id) = self.spawn_mob_raw("minecraft:allay", pos, true)
                && let Some(c) = self.common.mobs.iter_mut().find(|m| m.id == id)
            {
                c.extra.egg_time = 6000;
            }
            // Allay.duplicateAllay: entity event 18, three spawnHeartParticle.
            let id = self.common.mobs[index].id;
            self.mob_event_particles(id, 18);
            self.play_allay_sound("minecraft:block.amethyst_block.chime", at, id);
            self.consume_held();
            return true;
        }
        let acting = self.acting_uuid();
        let holds = m.extra.main_hand_id().is_some();
        if !holds && let Some(mut given) = item.cloned() {
            let m = &mut self.common.mobs[index];
            // `Allay.mobInteract` hands over one item, its components and all.
            given.count = 1;
            m.extra.set_main_hand_stack(Some(given));
            // `Allay.mobInteract` remembers LIKED_PLAYER by UUID.
            allay::like_player(m, acting);
            let id = m.id;
            self.play_allay_sound("minecraft:entity.allay.item_given", at, id);

            self.consume_held();
            return true;
        }
        // Allay.mobInteract takes the item back only through an empty main hand.
        if holds && item.is_none() && self.common.hand == Hand::Main {
            let m = &mut self.common.mobs[index];
            // The hand's stack keeps the components (a potion) the items came with.
            let held = m
                .extra
                .main_hand_stack()
                .cloned()
                .unwrap_or_else(|| ItemStack::new("minecraft:air", 1));
            m.extra.set_main_hand(None);
            let carried = m.extra.allay.as_mut().and_then(|st| st.inventory.take());
            allay::forget_liked_player(m);
            let id = m.id;
            self.play_allay_sound("minecraft:entity.allay.item_taken", at, id);

            // Allay.mobInteract ends with swing(MAIN_HAND, ...).
            self.common.mobs[index].swing();
            if let Some(stack) = carried {
                self.spawn_item(stack, at, [0.0, 0.2, 0.0], 40);
            }
            if let Some(rest) = self.add_stack_to_inventory(ItemStack { count: 1, ..held })
                && let Some(eye) = self.common.eye_position()
            {
                self.spawn_item(rest, eye, [0.0; 3], 0);
            }
            return true;
        }
        false
    }
}

impl AllayLevel for ServerGame<'_> {
    fn level_player_uuid(&self) -> Option<[i32; 4]> {
        self.common.level.player_uuid
    }
    fn host_player(&self) -> Option<AllayPlayer> {
        self.player_view().map(|p| AllayPlayer {
            position: p.position,
            eye: p.eye,
            spectator: p.spectator,
        })
    }
    fn guest_player(&self, uuid: [i32; 4]) -> Option<AllayPlayer> {
        self.lan_present()
            .into_iter()
            .find(|p| p.uuid == uuid && !p.spectator)
            .map(|p| AllayPlayer {
                position: sunrise_core::geometry::Vec3::new(
                    p.position[0],
                    p.position[1],
                    p.position[2],
                ),
                eye: p.eye,
                spectator: false,
            })
    }
    fn dimension_id(&self) -> &'static str {
        self.common.dim.id()
    }
    fn play_mob_bound_sound(
        &mut self,
        index: usize,
        event: &str,
        source: usize,
        volume: f32,
        pitch: f32,
        except_actor: bool,
    ) {
        crate::game::ServerGame::play_mob_bound_sound(
            self,
            index,
            event,
            source,
            volume,
            pitch,
            except_actor,
        );
    }
    fn with_sight_view<R>(&self, scan: impl FnOnce(SightView<'_>) -> R) -> R {
        let w = self.mob_world_base(&[]);
        scan(SightView {
            mobs: &self.common.mobs,
            dropped: &self.common.dropped,
            line_of_sight: &|from_eye, to| w.line_of_sight(from_eye, to),
        })
    }
    fn trigger_block(&mut self, uuid: [i32; 4], name: &str, pos: [i32; 3], stack: &ItemStack) {
        self.for_each_player(|g| {
            if g.acting_uuid() == uuid && !g.spectator() {
                g.trigger_block(name, pos, stack);
            }
        });
    }
    fn spawn_item(
        &mut self,
        stack: ItemStack,
        position: [f64; 3],
        velocity: [f64; 3],
        pickup_delay: i32,
    ) {
        crate::game::ServerGame::spawn_item(self, stack, position, velocity, pickup_delay);
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::item_use_stacks::ItemUseStacksGameExt1 as _;
