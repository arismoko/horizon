//! Leads on the game side: putting them on and taking them off mobs
//! (`Entity.interact`'s leash part), fence knots (`LeadItem.useOn` and
//! `LeashFenceKnotEntity.interact`), loading and saving who holds what,
//! breaking, and drawing the rope (`LeashFeatureRenderer`) and the knot.
use super::commands_common::Target;
use mod_api::LootDrops;
use sunrise_core::nbt::Tag;
use sunrise_gameplay_common::game::item_abilities::ability::SHEARS_HARVEST;
#[allow(unused_imports)]
pub(in crate::game) use sunrise_gameplay_common::leash_knot::{
    Knot, held_by_acting_player, resolve_pending_leash, write_mob_leash_tag,
};
use sunrise_items::items::ItemStack;
use sunrise_sim::mobs::leash::{self, Holder};

/// The item context of a leash success (`InteractionResult.Success`): tying
/// held mobs to this one and the holder's detach are `withoutItem`, so item
/// criteria see an empty hand; shears and a new lead keep the used item.
pub(in crate::game) enum LeashInteraction {
    WithItem,
    WithoutItem,
}

impl crate::game::ServerGame<'_> {
    pub(in crate::game::server_side) fn lead_sound(&mut self, sound: &str, at: [f64; 3]) {
        self.server_play_sound(&format!("minecraft:item.lead.{sound}"), at, 1.0, 1.0);
    }
    /// The fences the live leads of the level in place hang on, mobs first,
    /// each once.
    pub(in crate::game::server_side) fn lead_fences(&self) -> Vec<[i32; 3]> {
        let mut fences = Vec::new();
        let mobs = self
            .common
            .mobs
            .iter()
            .filter(|m| !m.removed)
            .map(|m| m.extra.leash);
        for holder in mobs.chain(self.common.boats.iter().map(|b| b.leash)) {
            if let Some(Holder::Knot(p)) = holder
                && !fences.contains(&p)
            {
                fences.push(p);
            }
        }
        fences
    }
    /// `LeashFenceKnotEntity.notifyLeasheeRemoved`: a fence no lead hangs on
    /// any more has lost its knot, with its id. Its UUID goes with the rest
    /// of the dead entities' memory (`prune_entity_memory`).
    pub fn forget_untied_knots(&mut self) {
        let fences = self.lead_fences();
        self.server.knot_ids.retain(|p, _| fences.contains(p));
        self.common
            .resident_entities
            .knots
            .retain(|knot| fences.contains(&knot.fence));
    }
    /// `LeashFenceKnotEntity.notifyLeasheeRemoved`: only a knot losing a
    /// holder asks whether it still has another one. The departing entity may
    /// be ticked through a copy, so its old stored payload must not count.
    pub(in crate::game::server_side) fn notify_knot_leashee_removed(
        &mut self,
        holder: Option<Holder>,
        departing: u32,
    ) {
        let Some(Holder::Knot(fence)) = holder else {
            return;
        };
        if self
            .common
            .mobs
            .iter()
            .any(|m| m.id != departing && !m.removed && m.extra.leash == Some(Holder::Knot(fence)))
            || self
                .common
                .boats
                .iter()
                .any(|b| b.id != departing && b.leash == Some(Holder::Knot(fence)))
        {
            return;
        }
        self.server.knot_ids.remove(&fence);
        self.common
            .resident_entities
            .knots
            .retain(|knot| knot.fence != fence);
    }
    /// `LeashFenceKnotEntity.getOrCreateKnot` → `createKnot`: the first lead
    /// tied to a fence makes its knot, a real entity with its own id (the
    /// shared `Entity.id` counter) and UUID, whether or not anyone tracks
    /// it. A fence that already has one keeps it. Call
    /// [`Self::forget_untied_knots`] before the tie.
    pub fn tie_knot(&mut self, fence: [i32; 3]) -> u32 {
        if let Some(&id) = self.server.knot_ids.get(&fence) {
            return id;
        }
        self.server.next_entity += 1;
        let id = self.server.next_entity;
        self.server.knot_ids.insert(fence, id);
        self.common.resident_entities.knots.push(Knot {
            attachment_lifetime: Default::default(),
            id,
            join: Default::default(),
            fence,
            fire_ticks: 0,
            invulnerable: false,
            no_gravity: false,
            silent: false,
            glowing: false,
        });
        self.uuid_for(id);
        id
    }
    /// The authoritative fence of an active natural knot, until a last-holder
    /// notification or the tracking cleanup retires its physical owner.
    pub fn natural_knot(&self, id: u32) -> Option<[i32; 3]> {
        use sunrise_sim::join_seq::residency::table::EntityLookup;
        let EntityLookup::Found {
            location,
            value: Target::Knot(_),
        } = self.server_resident_entity(id)
        else {
            return None;
        };
        if location.owner.dimension != self.common.dim.key() {
            return None;
        }
        let index = self.common.resident_entities.knots.locate(location.join)?;
        Some(self.common.resident_entities.knots[index].fence)
    }

    /// `BlockAttachedEntity.kill`: retire a natural knot before notifying its holders.
    /// Unlike vanilla's object references, this level's leash links name the fence.
    pub(in crate::game) fn kill_natural_knot(&mut self, id: u32) -> bool {
        let Some(fence) = self.natural_knot(id) else {
            return false;
        };
        let holder = Some(Holder::Knot(fence));
        let mobs: Vec<_> = self
            .common
            .mobs
            .iter()
            .filter(|mob| !mob.removed && mob.extra.leash == holder)
            .map(|mob| mob.id)
            .collect();
        let boats: Vec<_> = self
            .common
            .boats
            .iter()
            .filter(|boat| boat.leash == holder)
            .map(|boat| boat.id)
            .collect();
        self.server.knot_ids.remove(&fence);
        self.common
            .resident_entities
            .knots
            .retain(|knot| knot.id != id);
        for id in mobs {
            if self
                .common
                .mobs
                .iter()
                .any(|mob| mob.id == id && mob.extra.leash == holder)
            {
                self.detach_leash(Target::Mob(id), true);
            }
        }
        for id in boats {
            if self
                .common
                .boats
                .iter()
                .any(|boat| boat.id == id && boat.leash == holder)
            {
                self.detach_leash(Target::Boat(id), true);
            }
        }
        self.native_entity_left(id);
        true
    }
    /// Actual natural knot owners, without allocating or inferring membership from leashes.
    pub(in crate::game::server_side) fn natural_knot_id_iter(
        &self,
    ) -> impl Iterator<Item = u32> + '_ {
        self.common
            .resident_entities
            .knots
            .iter()
            .map(|knot| knot.id)
    }
    /// The ids of the level's natural knots that a lead still holds, in the
    /// order they were made.
    pub(in crate::game::server_side) fn natural_knot_ids(&self) -> Vec<u32> {
        let mut ids: Vec<u32> = self.natural_knot_id_iter().collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }
    /// BlockAttachedEntity.tick / LeashFenceKnotEntity.survives for explicitly loaded knots.
    pub(in crate::game::server_side) fn tick_explicit_knots(&mut self) {
        let ticking = self.entity_ticking_area();
        let world = &self.common.world;
        let facts = self.common.res.facts();
        let fences = self.common.res.items().block_tags.get("minecraft:fences");
        let mut untied = Vec::new();
        self.server.others.0.retain_mut(|knot| {
            let Some(check) = knot.knot_check.as_mut() else {
                return true;
            };
            let at = sunrise_core::position::BlockPos::containing(knot.position);
            if !ticking(at.x, at.z) || !world.chunks.contains_key(&(at.x >> 4, at.z >> 4)) {
                return true;
            }
            // Vanilla compares the old postincrement value: first check is tick 101.
            if *check >= 100 {
                *check = 0;
                let state = facts.get(world.block(at.x, at.y, at.z));
                if !fences.is_some_and(|f| f.contains(&*state.name)) {
                    untied.push(knot.position.components());
                    return false;
                }
            } else {
                *check += 1;
            }
            true
        });
        // LeashFenceKnotEntity.dropItem plays LEAD_UNTIED; it drops no lead item.
        for at in untied {
            self.lead_sound("untied", at);
        }
    }
    /// `Leashable.canHaveALeashAttachedTo` for the mob at `index` and a
    /// holder whose box centre is `center`: within the species' snapping
    /// range and leashable.
    pub fn can_attach_lead(&self, index: usize, center: glam::DVec3) -> bool {
        leash::can_have_a_leash_attached(
            &self.common.mobs[index],
            sunrise_core::geometry::Vec3::new(center.x, center.y, center.z),
        )
    }
    /// `Entity.gameEvent(event, player)` for a lead: the acting player is the source.
    pub(in crate::game) fn lead_game_event(&mut self, event: &'static str, at: [f64; 3]) {
        let ctx = self.player_context(None);
        self.post_game_event(event, at, ctx);
    }
    /// `Entity.interact`'s sneak branch: the mobs and boats on the player's
    /// lead within reach of `center` that `canHaveALeashAttachedTo` the
    /// entity (`skip` is the entity itself) are re-tied to `to`. Whether any
    /// was.
    pub fn tie_player_leads(
        &mut self,
        center: glam::DVec3,
        to: Holder,
        skip: Option<usize>,
    ) -> bool {
        let knot_id = match to {
            Holder::Knot(fence) => self.server.knot_ids.get(&fence).copied(),
            _ => None,
        };
        let mut any = false;
        for j in self.player_leashed(center) {
            if Some(j) != skip && self.can_attach_lead(j, center) {
                self.common.mobs[j].extra.leash = Some(to);
                self.common.mobs[j].extra.leash_knot_id = knot_id;
                any = true;
            }
        }
        // The boats on the player's lead too (game/entities/boat_leads.rs).
        any | self.tie_player_boats(center, to)
    }
    pub(in crate::game) fn player_center(&self) -> glam::DVec3 {
        self.common.player.as_ref().map_or(glam::DVec3::ZERO, |p| {
            glam::DVec3::new(
                p.position.x,
                p.position.y + f64::from(p.dimensions.height) / 2.0,
                p.position.z,
            )
        })
    }
    /// The mobs on the acting player's lead within 32 blocks of a point
    /// (`Leashable.leashableInArea` with `getLeashHolder() == player`).
    pub(in crate::game) fn player_leashed(&self, around: glam::DVec3) -> Vec<usize> {
        let acting = self.acting_uuid();
        (0..self.common.mobs.len())
            .filter(|&i| {
                let m = &self.common.mobs[i];
                held_by_acting_player(m.extra.leash, m.extra.leash_player, acting)
                    && (m.position.x - around.x).abs() <= 16.0
                    && (m.position.y - around.y).abs() <= 16.0
                    && (m.position.z - around.z).abs() <= 16.0
            })
            .collect()
    }
    /// `Entity.spawnAtLocation(level, Items.LEAD)`: the item entity draws its
    /// motion from its own random, not the level's.
    pub(in crate::game::server_side) fn drop_lead_item(&mut self, at: [f64; 3]) {
        self.spawn_item_own(ItemStack::new("minecraft:lead", 1), at, 10);
    }
    /// `Leashable.dropLeash` / `removeLeash`: the lead falls as an item.
    pub fn break_leash(&mut self, id: u32, drop: bool, sound: bool) {
        self.drop_leash(id, drop, sound, false);
    }
    /// `Leashable.tickLeash` ending a lead: a dying mob's falls as an item only under
    /// `entity_drops`, else it is `removeLeash`.
    pub(in crate::game::server_side) fn tick_break_leash(
        &mut self,
        id: u32,
        drop: bool,
        sound: bool,
    ) {
        let dying = self.common.mobs.iter().any(|m| m.id == id && !m.alive());
        let drop = drop
            && (!dying
                || self
                    .common
                    .ui
                    .rules
                    .bool_at(sunrise_game_data::gamerules::vanilla::ENTITY_DROPS));
        self.break_leash(id, drop, sound);
    }
    /// `Entity.dropAllLeashConnections`: every mob and boat leashed to the
    /// acting player, inside the 32-block box around the player's centre,
    /// drops its lead. Then `GameEvent.SHEAR`. Whether anything dropped.
    /// The player is not `Leashable`, so there is no lead of their own.
    pub(in crate::game::server_side) fn drop_all_leash_connections(&mut self) -> bool {
        let center = self.player_center();
        let mobs: Vec<u32> = self
            .player_leashed(center)
            .into_iter()
            .map(|i| self.common.mobs[i].id)
            .collect();
        let boats = self.player_led_boats(center);
        if mobs.is_empty() && boats.is_empty() {
            return false;
        }
        for id in mobs {
            // One snap is played by the caller (`SoundEvents.LEAD_BREAK`),
            // not a break sound on each leashed entity.
            self.break_leash(id, true, false);
        }
        self.drop_boat_leashes(&boats);
        let at = self
            .common
            .player
            .as_ref()
            .map_or([center.x, center.y, center.z], |p| {
                [p.position.x, p.position.y, p.position.z]
            });
        // `gameEvent(GameEvent.SHEAR, null)`: the player is the position, not the source.
        let ctx = self.who_context(None, None);
        self.post_game_event("minecraft:shear", at, ctx);
        true
    }
    /// `Leashable.setLeashedTo` on the mob or boat `target`: its lead goes to `holder` whatever
    /// held it, or was still waiting for a saved holder. `player` is a player holder's UUID.
    /// Taking the lead over is no removal, so no unleash hook hears it. False for anything else.
    pub fn set_leash(&mut self, target: Target, holder: Holder, player: Option<[i32; 4]>) -> bool {
        let knot_id = match holder {
            Holder::Knot(fence) => self.server.knot_ids.get(&fence).copied(),
            _ => None,
        };
        match target {
            Target::Mob(id) => {
                let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id) else {
                    return false;
                };
                let old = m.extra.leash;
                m.extra.leash = Some(holder);
                m.extra.leash_knot_id = knot_id;
                m.extra.leash_player = player;
                m.extra.leash_pending = None;
                if old != Some(holder) {
                    self.notify_knot_leashee_removed(old, id);
                }
                true
            }
            Target::Boat(id) => {
                let Some(i) = self.common.boats.iter().position(|b| b.id == id) else {
                    return false;
                };
                let old = self.common.boats[i].leash;
                self.common.boats.update(i, |b| {
                    b.leash = Some(holder);
                    b.leash_knot_id = knot_id;
                    b.leash_player = player;
                    b.leash_pending = None;
                });
                if old != Some(holder) {
                    self.notify_knot_leashee_removed(old, id);
                }
                true
            }
            _ => false,
        }
    }
    /// `Leashable.dropLeash` (`drop`) or `removeLeash` on the mob or boat `target`. False when no
    /// holder has the lead: a holder still unresolved from a save stays, as in `dropLeash`.
    pub fn detach_leash(&mut self, target: Target, drop: bool) -> bool {
        match target {
            Target::Mob(id) => {
                let held = self
                    .common
                    .mobs
                    .iter()
                    .any(|m| m.id == id && m.extra.leash.is_some());
                if held {
                    self.break_leash(id, drop, false);
                }
                held
            }
            Target::Boat(id) => {
                let Some(i) = self.common.boats.iter().position(|b| b.id == id) else {
                    return false;
                };
                let held = self.common.boats[i].leash.is_some();
                self.release_stored_boat_leash(i, drop);
                held
            }
            _ => false,
        }
    }
    /// `break_leash` by the acting player's use (untying it, shears).
    pub(in crate::game) fn break_leash_used(&mut self, id: u32, drop: bool) {
        self.drop_leash(id, drop, false, true);
    }
    pub(in crate::game::server_side) fn drop_leash(
        &mut self,
        id: u32,
        drop: bool,
        sound: bool,
        used: bool,
    ) {
        let killer = self
            .server
            .lead_killers
            .remove(&id)
            .map(|(killer, _)| killer);
        let Some((at, kind, holder)) =
            self.common
                .mobs
                .iter_mut()
                .find(|m| m.id == id)
                .and_then(|m| {
                    let holder = m.extra.leash;
                    sunrise_sim::mobs::leash::remove(m)?;
                    Some((
                        [m.position.x, m.position.y, m.position.z],
                        sunrise_gameplay_common::game::native_hooks::mob_kind(m),
                        holder,
                    ))
                })
        else {
            return;
        };
        self.notify_knot_leashee_removed(holder, id);
        self.native_entity_unleash(id, &kind);
        if used
            && let (Some(player), Some(name)) = (
                self.server.lan_acting_uuid,
                self.server.lan_acting_name.clone(),
            )
        {
            self.native_player_unleash_entity(player, &name, id);
        }
        if sound {
            self.lead_sound("break", at);
        }
        if !drop {
            return;
        }
        // The mods' say when a player is behind it: the user, else the
        // killer of a mob that died leashed.
        let player = if used {
            Some(self.native_acting_id())
        } else {
            killer
        };
        let Some(player) = player else {
            self.drop_lead_item(at);
            return;
        };
        let mut drops = vec![ItemStack::new("minecraft:lead", 1)];
        let context = LootDrops::from(mod_api::__private::built::LootDrops {
            weapon: None,
            damage_source: None,
            table: "",
            dimension: self.common.dim.id(),
            origin: at,
            block: None,
            entity: Some((id, &kind)),
            player: Some(player),
            tool: None,
            explosion: None,
            used,
        });
        self.native_loot_drops(&context, &mut drops);
        for stack in drops {
            if stack.count > 0 && &*stack.item != "minecraft:air" {
                self.spawn_item_own(stack, at, 10);
            }
        }
    }
    /// `LeashFenceKnotEntity.hurtServer` -> `kill` and `dropItem` for a natural or explicit knot.
    pub(in crate::game::server_side) fn break_leash_knot(
        &mut self,
        id: u32,
        source: Option<Target>,
    ) -> bool {
        if let Some(fence) = self.natural_knot(id) {
            let at = leash::knot_state(fence).position.components();
            self.server.knot_ids.remove(&fence);
            self.common.resident_entities.knots.retain(|k| k.id != id);
            if !self.server.broken_knots.contains(&fence) {
                self.server.broken_knots.push(fence);
            }
            self.entity_killed_event_by(at, source);
            self.lead_sound("untied", at);
            return true;
        }
        if let Some(i) = self
            .server
            .others
            .0
            .iter()
            .position(|o| o.id == id && o.kind == "minecraft:leash_knot")
        {
            let knot = self.server.others.0.remove(i);
            let at = knot.position.components();
            self.entity_killed_event_by(at, source);
            self.lead_sound("untied", at);
            return true;
        }
        false
    }
    /// Holders read from a save become real once the entity is found; a
    /// holder that never shows up drops the lead after 100 ticks.
    pub fn tick_leads(&mut self) {
        if !self.server.broken_knots.is_empty() {
            let fences = std::mem::take(&mut self.server.broken_knots);
            let drop = self
                .common
                .ui
                .rules
                .bool_at(sunrise_game_data::gamerules::vanilla::ENTITY_DROPS);
            for fence in fences {
                let live_id = self.server.knot_ids.get(&fence).copied();
                let mobs: Vec<u32> = self
                    .common
                    .mobs
                    .iter()
                    .filter(|m| {
                        !m.removed
                            && m.extra.leash == Some(Holder::Knot(fence))
                            && (live_id.is_none() || m.extra.leash_knot_id != live_id)
                    })
                    .map(|m| m.id)
                    .collect();
                for mob_id in mobs {
                    self.break_leash(mob_id, drop, false);
                }
                let boats: Vec<usize> = self
                    .boats_on(Holder::Knot(fence))
                    .into_iter()
                    .filter(|&i| live_id.is_none() || self.common.boats[i].leash_knot_id != live_id)
                    .collect();
                for boat_idx in boats {
                    self.release_stored_boat_leash(boat_idx, drop);
                }
            }
        }
        // A corpse gone without dropping its lead takes its killer along.
        // Only this level's: the others' mobs aren't in place.
        if !self.server.lead_killers.is_empty() {
            let (mobs, dim) = (&self.common.mobs, self.common.dim);
            self.server
                .lead_killers
                .retain(|id, (_, at)| *at != dim || mobs.iter().any(|m| m.id == *id && !m.removed));
        }
        let player_uuid = self.common.level.player_uuid.unwrap_or([0, 0, 0, 1]);
        let owner_uuid = self
            .server
            .serving
            .as_ref()
            .map(|s| s.owner)
            .or_else(|| {
                self.server
                    .lan
                    .as_ref()
                    .and_then(|h| h.hub.singleplayer_owner)
            })
            .map(sunrise_sim::lan::playerdata::uuid_ints);
        for i in 0..self.common.mobs.len() {
            let Some(pending) = self.common.mobs[i].extra.leash_pending else {
                continue;
            };
            let holder = resolve_pending_leash(
                pending,
                player_uuid,
                owner_uuid,
                |u| {
                    self.server.lan.as_ref().is_some_and(|h| {
                        h.hub
                            .guests
                            .iter()
                            .any(|g| sunrise_sim::lan::playerdata::uuid_ints(g.uuid) == u)
                    })
                },
                |u| self.mob_by_uuid(u),
                |u| self.boat_by_uuid(u),
                |u| self.knot_by_uuid(u),
            );
            if let Some((Holder::Knot(_), _)) = holder {
                self.forget_untied_knots();
            }
            if let Some((h, player)) = holder {
                let knot_id = if let Holder::Knot(fence) = h {
                    Some(self.tie_knot(fence))
                } else {
                    None
                };
                let m = &mut self.common.mobs[i];
                if h == Holder::Player {
                    m.extra.leash_player = player;
                }
                m.extra.leash = Some(h);
                m.extra.leash_knot_id = knot_id;
                m.extra.leash_pending = None;
            } else if self.common.mobs[i].age > 100 {
                self.common.mobs[i].extra.leash_pending = None;
                let at = [
                    self.common.mobs[i].position.x,
                    self.common.mobs[i].position.y,
                    self.common.mobs[i].position.z,
                ];
                self.drop_lead_item(at);
            }
        }
    }
    /// The `leash` tag of a mob about to be saved.
    pub(in crate::game::server_side) fn write_leash(&mut self, tag: &mut Tag, id: u32) {
        let Some(m) = self.common.mob_object(id) else {
            return;
        };
        let (holder, pending, leash_player) =
            (m.extra.leash, m.extra.leash_pending, m.extra.leash_player);
        let level_player_uuid = self.common.level.player_uuid;
        write_mob_leash_tag(
            tag,
            holder,
            pending,
            leash_player,
            level_player_uuid,
            |id| self.mob_uuid(id),
        );
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn interact_leash(&mut self, index: usize, item: &str) -> bool {
        self.interact_leash_with_item_context(index, item).is_some()
    }
    /// `LeadItem.useOn` on the fence the crosshair hits.
    pub(in crate::game::server_side) fn interact_knot(&mut self) -> bool {
        let Some(hit) = self.pick() else {
            return false;
        };
        let name = self
            .common
            .res
            .facts()
            .get(self.common.world.block(hit.pos[0], hit.pos[1], hit.pos[2]));
        if !self
            .common
            .res
            .items()
            .block_tags
            .get("minecraft:fences")
            .is_some_and(|s| s.contains(&*name.name))
        {
            return false;
        }
        // `FenceBlock.useWithoutItem` / `LeadItem.useOn` only tie the player's own mobs
        // (`LeadItem.bindPlayerMobs`). A client whose crosshair met the knot sends Interact with
        // the knot's id instead. Taking a mob back here would undo a tie: the client sends the
        // click once for each hand.
        self.use_knot(hit.pos, false)
    }
}

impl crate::game::ServerGame<'_> {
    /// `Entity.interact`: retain the holder-detach success's item context.
    pub(in crate::game) fn interact_leash_with_item_context(
        &mut self,
        index: usize,
        item: &str,
    ) -> Option<LeashInteraction> {
        let sneaking = self.server_acting_sneak();
        let id = self.common.mobs[index].id;
        let at = self.common.mobs[index].center();
        if !self.common.mobs[index].alive() {
            return None;
        }
        // Sneaking with mobs in hand: tie them to this one.
        if sneaking
            && leash::can_be_leashed(&self.common.mobs[index])
            && !self.common.mobs[index].is_baby()
        {
            let b = self.common.mobs[index].bounds();
            let (lo, hi) = (b.min_position(), b.max_position());
            let center = glam::DVec3::new(
                (lo.x + hi.x) / 2.0,
                (lo.y + hi.y) / 2.0,
                (lo.z + hi.z) / 2.0,
            );
            if self.tie_player_leads(center, Holder::Mob(id), Some(index)) {
                self.lead_game_event("minecraft:entity_action", at);
                self.lead_sound("tied", at);
                return Some(LeashInteraction::WithoutItem);
            }
        }
        if self.held_shears(SHEARS_HARVEST) {
            let mut dropped = false;
            if self.common.mobs[index].extra.leash.is_some() {
                self.break_leash_used(id, true);
                dropped = true;
            }
            let others: Vec<u32> = self
                .common
                .mobs
                .iter()
                .filter(|m| m.extra.leash == Some(Holder::Mob(id)))
                .map(|m| m.id)
                .collect();
            for o in others {
                self.break_leash_used(o, true);
                dropped = true;
            }
            let boats = self.boats_on(Holder::Mob(id));
            dropped |= !boats.is_empty();
            self.drop_boat_leashes(&boats);
            if dropped {
                self.lead_game_event("minecraft:shear", at);
                self.server_play_sound("minecraft:item.shears.snip", at, 1.0, 1.0);
                self.damage_held_tool();
                return Some(LeashInteraction::WithItem);
            }
        }
        // Mob.checkAndHandleImportantInteractions: only the player the lead
        // is tied to unties it (`getLeashHolder() == player`).
        let acting = self.acting_player_uuid();
        if held_by_acting_player(
            self.common.mobs[index].extra.leash,
            self.common.mobs[index].extra.leash_player,
            acting,
        ) {
            let creative = self.common.creative;
            self.break_leash_used(id, !creative);
            self.lead_game_event("minecraft:entity_interact", at);
            self.lead_sound("untied", at);
            return Some(LeashInteraction::WithoutItem);
        }
        if item == "minecraft:lead" {
            let center = self.player_center();
            if self.can_attach_lead(index, center) {
                if self.common.mobs[index].extra.leash.is_some() {
                    self.break_leash_used(id, true);
                }
                let holder = self.acting_player_uuid();
                // LeashData.setLeashHolder: a holder still waiting in the save
                // is forgotten, so it can neither take over nor drop a lead.
                let extra = &mut self.common.mobs[index].extra;
                extra.leash = Some(Holder::Player);
                extra.leash_pending = None;
                extra.leash_player = Some(holder);
                self.lead_sound("tied", at);
                // `Player.interactOn` restores the count of a player with
                // infinite materials (creative); others spend the lead.
                self.consume_held();
                return Some(LeashInteraction::WithItem);
            }
        }
        None
    }
    /// A click at the fence at `knot_pos`: `knot_hit` for `LeashFenceKnotEntity.interact`,
    /// otherwise `LeadItem.useOn`. Returns whether it was used.
    pub fn use_knot(&mut self, knot_pos: [i32; 3], knot_hit: bool) -> bool {
        let knot = leash::knot_state(knot_pos).center;
        let center = glam::DVec3::new(knot.x, knot.y, knot.z);
        let at = [center.x, center.y, center.z];
        let existing: Vec<usize> = (0..self.common.mobs.len())
            .filter(|&i| self.common.mobs[i].extra.leash == Some(Holder::Knot(knot_pos)))
            .collect();
        let existing_boats = self.boats_on(Holder::Knot(knot_pos));
        let held_item = self.common.held().map(|s| s.item.to_string());
        let sneaking = self.server_acting_sneak();
        // Shears snip every lead on the knot.
        if knot_hit && self.held_shears(SHEARS_HARVEST) {
            for i in existing {
                let id = self.common.mobs[i].id;
                self.break_leash_used(id, true);
            }
            self.drop_boat_leashes(&existing_boats);
            self.server_play_sound("minecraft:item.shears.snip", at, 1.0, 1.0);
            self.damage_held_tool();
            return true;
        }
        let in_hand = self.player_leashed(center);
        let boats_in_hand = self.player_led_boats(center);
        // The knot is only made for the lead item, or when mobs already hang on it.
        if held_item.as_deref() != Some("minecraft:lead")
            && existing.is_empty()
            && existing_boats.is_empty()
            && in_hand.is_empty()
            && boats_in_hand.is_empty()
        {
            return false;
        }
        // A knot is made when a lead is tied to a fence that has none.
        if existing.is_empty()
            && existing_boats.is_empty()
            && (!in_hand.is_empty() || !boats_in_hand.is_empty())
            && self.hanging_place_cancelled("minecraft:leash_knot", knot_pos)
        {
            return true;
        }
        self.forget_untied_knots();
        let mut attached = self.tie_player_boats(center, Holder::Knot(knot_pos));
        let mut attached_mobs = Vec::new();
        for j in in_hand {
            if self.can_attach_lead(j, center) {
                self.common.mobs[j].extra.leash = Some(Holder::Knot(knot_pos));
                attached_mobs.push(j);
                attached = true;
            }
        }
        if attached {
            let knot_id = self.tie_knot(knot_pos);
            for j in attached_mobs {
                self.common.mobs[j].extra.leash_knot_id = Some(knot_id);
            }
            for i in self.boats_on(Holder::Knot(knot_pos)) {
                if self.common.boats[i].leash_knot_id.is_none() {
                    self.common
                        .boats
                        .update(i, |b| b.leash_knot_id = Some(knot_id));
                }
            }
        }
        if !attached && !sneaking && knot_hit {
            let player = self.player_center();
            let holder = self.acting_player_uuid();
            for i in existing {
                if self.can_attach_lead(i, player) {
                    self.common.mobs[i].extra.leash = Some(Holder::Player);
                    self.common.mobs[i].extra.leash_player = Some(holder);
                    attached = true;
                }
            }
            attached |= self.take_knot_boats(knot_pos);
        }
        if attached {
            let event_at = if knot_hit {
                [knot.x, knot.y - 0.25, knot.z]
            } else {
                knot_pos.map(|v| f64::from(v) + 0.5)
            };
            self.lead_game_event("minecraft:block_attach", event_at);
            self.lead_sound("tied", at);
            self.swing_arm();
        }
        attached
    }
}

use sunrise_gameplay_common::game::side_ops::presentation::PresentationSideGameExt as _;

use sunrise_gameplay_common::game::item_use_stacks::ItemUseStacksGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::ModGameExt1 as _;

use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;
