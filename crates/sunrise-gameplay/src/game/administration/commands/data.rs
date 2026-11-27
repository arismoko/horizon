//! `/data` (DataCommands), `/tag` (TagCommand) and `/summon` with NBT: entity,
//! block entity and storage data accessors over the game state.
use super::{Arg, Context, R, Target, err};
use sunrise_core::nbt::{Compound, JavaString, Tag, parse_snbt};
use sunrise_game_data::{nbt_path::NbtPath, snbt, text::Text};
use sunrise_sim::{
    data_cmd::{self, Op, path_error},
    entity_nbt,
    save::{self},
    scoreboard_cmd::{Holder, Tracker, format_list},
};

/// A `DataAccessor`.
#[derive(Clone, Debug, PartialEq)]
pub enum DataRef {
    Entity(Target),
    Block([i32; 3]),
    Storage(String),
}

pub(in crate::game) fn empty() -> Tag {
    Tag::Compound(Compound::new())
}
pub fn set_key(tag: &mut Tag, key: &str, value: Tag) {
    if let Tag::Compound(c) = tag {
        c.insert(JavaString::from(key), value);
    }
}
/// `SummonCommand.spawnEntity`'s name for a kind no command target stands for:
/// its entity type's. An item is the exception: `EntityType`'s factory leaves
/// `ItemStack.EMPTY`, which `ItemEntity.readAdditionalSaveData` discards, and
/// `ItemEntity.getName` reads that empty stack's `getItemName` — an empty
/// component, not the type's name.
pub(in crate::game) fn summon_type_name(kind: &str) -> Text {
    if kind.trim_start_matches("minecraft:") == "item" {
        return Text::empty();
    }
    Text::translate(&super::entity_name_key(kind), &[])
}
/// `EntityType.loadEntityRecursive` snaps every entity of the stack,
/// passengers included, to the position (`/summon`, `summon`).
pub(in crate::game) fn place_summoned(tag: &mut Tag, pos: [f64; 3]) {
    let p = pos.map(|v| Tag::Double(v.to_bits()));
    set_key(tag, "Pos", Tag::List(p.to_vec()));
    // Entity.load: an absent Fire reads as 0 (until the entity moves).
    if tag.get("Fire").is_none() {
        set_key(tag, "Fire", Tag::Short(0));
    }
    if tag.get("Rotation").is_none() {
        set_key(
            tag,
            "Rotation",
            Tag::List(vec![
                Tag::Float(0.0f32.to_bits()),
                Tag::Float(0.0f32.to_bits()),
            ]),
        );
    }
    if let Tag::Compound(c) = tag
        && let Some(Tag::List(riders)) = c.get_mut(&"Passengers".into())
    {
        for r in riders {
            if matches!(r, Tag::Compound(_)) {
                place_summoned(r, pos);
            }
        }
    }
}

/// `Level.isInSpawnableBounds(BlockPos.containing(pos))`.
pub(in crate::game) fn summon_position_ok([x, y, z]: [f64; 3]) -> bool {
    (-30_000_000.0..30_000_000.0).contains(&x)
        && (-20_000_000.0..20_000_000.0).contains(&y)
        && (-30_000_000.0..30_000_000.0).contains(&z)
}

impl crate::game::ServerGame<'_> {
    // ---- entity data ---------------------------------------------------

    /// `NbtPredicate.getEntityTagToCompare`: the entity as saved. An entity
    /// in another level is read in the level that holds it.
    pub fn command_target_nbt(&mut self, t: Target) -> Option<Tag> {
        self.in_target_level(t, |g| g.entity_nbt_in_level(t))
            .flatten()
    }
    /// [`Self::entity_nbt`] of the player whose turn it is.
    pub fn player_nbt(&mut self) -> Option<Tag> {
        self.common.player.as_ref()?;
        self.sync_level();
        let mut tag = self.common.level.player_tag(self.common.res.items());
        if let Some(u) = self.saved_player_uuid() {
            set_key(&mut tag, "UUID", Tag::IntArray(u.to_vec()));
        }
        // Entity.saveWithoutId: the live `portalCooldown`, the player's own on its turn.
        set_key(
            &mut tag,
            "PortalCooldown",
            Tag::Int(self.common.portal_player.cooldown),
        );
        // NbtPredicate.getEntityTagToCompare: a player's held stack.
        let selected = self.common.level.selected_slot;
        let held = match tag.get("Inventory") {
            Some(Tag::List(items)) => items
                .iter()
                .find(|i| matches!(i.get("Slot"), Some(Tag::Byte(b)) if i32::from(*b) == selected))
                .cloned(),
            _ => None,
        };
        if let Some(mut stack) = held {
            if let Tag::Compound(c) = &mut stack {
                c.remove(&"Slot".into());
            }
            set_key(&mut tag, "SelectedItem", stack);
        }
        Some(tag)
    }
    pub fn entity_nbt_in_level(&mut self, t: Target) -> Option<Tag> {
        self.entity_tag_in_level(t, true)
    }

    /// `Entity.load` with `AbstractArrow.readAdditionalSaveData` on the resident arrow or
    /// spectral arrow at `index` of the projectile list: the loader's decoding
    /// (`arrow_from_nbt`), over the entity it already is. Its type, id, join,
    /// random, UUID and tick count stay; so does what no save holds (its pierced
    /// entities, `inGroundTime`, fluid state). None when the data has no position.
    pub(in crate::game::server_side) fn load_resident_arrow(
        &mut self,
        index: usize,
        spectral: bool,
        tag: &Tag,
    ) -> Option<()> {
        // `Entity.load` cannot change the type, and `/data`'s compound has no `id`.
        let mut tag = tag.clone();
        let kind = if spectral {
            "minecraft:spectral_arrow"
        } else {
            "minecraft:arrow"
        };
        set_key(&mut tag, "id", Tag::String(kind.into()));
        let (mut new, in_block) = entity_nbt::arrow_from_nbt(
            &tag,
            self.saved_player_uuid(),
            crate::game::dispenser::tipped_effect,
        )?;
        // Projectile.readAdditionalSaveData: `setOwner(EntityReference.read(..))`.
        self.claim_owner(&mut new);
        let last_state = in_block.and_then(|t| self.state_from_nbt(&t));
        let read = new.arrow.take()?;
        self.common.creatures.projectiles.update(index, |p| {
            p.kind = new.kind;
            p.position = new.position;
            // Entity.load: setOldPosAndRot.
            p.previous = new.position;
            p.velocity = new.velocity;
            (p.yaw, p.pitch) = (new.yaw, new.pitch);
            (p.previous_yaw, p.previous_pitch) = (new.yaw, new.pitch);
            p.left_owner = new.left_owner;
            (p.owner, p.guest, p.owner_vehicle) = (new.owner, new.guest, new.owner_vehicle);
            p.owner_uuid = new.owner_uuid;
            p.no_gravity = new.no_gravity;
            p.ground.on_ground = new.ground.on_ground;
            p.air_supply = new.air_supply;
            p.stuck = new.stuck;
            let Some(a) = p.arrow.as_mut() else {
                return;
            };
            a.life = read.life;
            a.last_state = last_state;
            a.shake = read.shake;
            a.in_ground = read.in_ground;
            a.pickup = read.pickup;
            a.pierce = read.pierce;
            a.has_been_shot = read.has_been_shot;
            a.can_break = read.can_break;
            a.spectral_duration = read.spectral_duration;
            a.pickup_item = read.pickup_item;
            a.fired_from_weapon = read.fired_from_weapon;
            a.crossbow_hit_sound = read.crossbow_hit_sound;
            a.fire_ticks = read.fire_ticks;
            a.fall_distance = read.fall_distance;
        });
        Some(())
    }
    pub(in crate::game) fn raw_mut(&mut self, t: Target) -> Option<&mut Tag> {
        match t {
            Target::Player => Some(self.common.level.player_extra.get_or_insert_with(empty)),
            Target::Guest(conn) => self
                .server.lan
                .as_mut()?
                .hub
                .guests
                .saved_data_mut(conn)
                .map(|saved| saved.get_or_insert_with(empty)),
            Target::Mob(id)
            | Target::Item(id)
            | Target::Stand(id)
            | Target::Frame(id)
            | Target::Painting(id)
            | Target::Boat(id)
            | Target::Cart(id)
            // `tags_of` reads a primed TNT's compound; without this arm
            // `/tag` wrote nothing and still reported success.
            | Target::Tnt(id)
            | Target::Cushion(id)
            | Target::Orb(id)
            | Target::Projectile(id)
            | Target::Crystal(id)
            | Target::Dragon(id)
            | Target::Other(id)
            | Target::Fangs(id)
            | Target::Cloud(id)
            | Target::Bolt(id) => Some(self.common.persist.raw.entry(id).or_insert_with(empty)),
            // `tags_of` reads these two from the same map and a falling
            // block from its saved compound; without these arms `/tag`
            // wrote nothing and still reported success.
            Target::Rocket(id) | Target::Trident(id) => {
                Some(self.common.persist.raw.entry(id).or_insert_with(empty))
            }
            _ => None,
        }
    }
    /// Writes `t`'s `Tags`; false when its kind keeps no saved compound to
    /// write them to.
    pub fn set_entity_tags(&mut self, t: Target, tags: &[String]) -> bool {
        if let Target::Falling(id) = t {
            let uuid = self.uuid_for(id);
            let Some(i) = self.common.falling.iter().position(|f| f.id == id) else {
                return false;
            };
            self.common.falling.update(i, |f| {
                let raw = &mut f.saved.get_or_insert_with(|| Box::new((uuid, empty()))).1;
                set_key(
                    raw,
                    "Tags",
                    Tag::List(
                        tags.iter()
                            .map(|s| Tag::String(s.as_str().into()))
                            .collect(),
                    ),
                );
            });
            return true;
        }
        if let Target::Spawner(id) = t {
            let Some(i) = self.common.ominous_spawners.iter().position(|s| s.id == id) else {
                return false;
            };
            self.common.ominous_spawners.update(i, |spawner| {
                let raw = &mut **spawner.raw.get_or_insert_with(|| Box::new(empty()));
                if tags.is_empty() {
                    if let Tag::Compound(map) = raw {
                        map.remove(&"Tags".into());
                    }
                } else {
                    set_key(
                        raw,
                        "Tags",
                        Tag::List(
                            tags.iter()
                                .map(|s| Tag::String(s.as_str().into()))
                                .collect(),
                        ),
                    );
                }
            });
            return true;
        }
        let Some(raw) = self.raw_mut(t) else {
            return false;
        };
        // `Entity.saveWithoutId` writes `Tags` only for a nonempty live set:
        // removing the last tag leaves no key.
        if tags.is_empty() {
            if let Tag::Compound(map) = raw {
                map.remove(&"Tags".into());
            }
            return true;
        }
        let list = tags
            .iter()
            .map(|s| Tag::String(s.as_str().into()))
            .collect();
        set_key(raw, "Tags", Tag::List(list));
        true
    }

    /// `ComparatorBlockEntity.loadAdditional` (`getIntOr("OutputSignal", 0)`)
    /// for a comparator at `p`: None for any other block; else whether the
    /// loaded output differs from the one it had (None inside when the tag
    /// does not decode). The block's own metadata comes from `comparator_data`.
    pub(in crate::game::server_side) fn load_comparator_output(
        &mut self,
        p: [i32; 3],
        tag: &Tag,
    ) -> Option<Option<bool>> {
        let mut input = self.comparator_data(p)?;
        let before = input.get("OutputSignal").cloned();
        if let Some(value) = tag.get("OutputSignal") {
            set_key(&mut input, "OutputSignal", value.clone());
        } else if let Tag::Compound(map) = &mut input {
            map.remove(&"OutputSignal".into());
        }
        let Some(normalized) = sunrise_game_data::block_entity_nbt::canonical(&input) else {
            return Some(None);
        };
        let output = normalized.get("OutputSignal").cloned();
        if let Some(Tag::Int(value)) = output {
            let pos = sunrise_core::position::BlockPos::new(p[0], p[1], p[2]);
            self.common
                .block_ticks
                .comparator_outputs
                .insert(pos, value);
        }
        Some(Some(output != before))
    }

    /// `ComparatorBlockEntity.saveAdditional`: expose the circuit's live output,
    /// rather than keeping a second copy in the ordinary block entity map.
    pub(in crate::game::server_side) fn comparator_data(&self, p: [i32; 3]) -> Option<Tag> {
        if self.common.data_block_name(p) != "minecraft:comparator" {
            return None;
        }
        let pos = sunrise_core::position::BlockPos::new(p[0], p[1], p[2]);
        let output = self
            .common
            .block_ticks
            .comparator_outputs
            .get(&pos)
            .copied()
            .or_else(|| {
                let saved = self
                    .common
                    .world
                    .chunks
                    .get(&(p[0] >> 4, p[2] >> 4))?
                    .saved
                    .as_deref()?;
                let tag = saved.block_entities.iter().find(|t| {
                    [t.get("x"), t.get("y"), t.get("z")]
                        .map(|v| save::number(v).unwrap_or(0) as i32)
                        == p
                        && save::text(t.get("id")).as_deref() == Some("minecraft:comparator")
                })?;
                let tag = sunrise_game_data::block_entity_nbt::canonical(tag)?;
                match tag.get("OutputSignal") {
                    Some(Tag::Int(v)) => Some(*v),
                    _ => None,
                }
            })
            .unwrap_or(0);
        Some(Tag::compound([
            ("OutputSignal", Tag::Int(output)),
            ("components", empty()),
            ("id", Tag::String("minecraft:comparator".into())),
            ("x", Tag::Int(p[0])),
            ("y", Tag::Int(p[1])),
            ("z", Tag::Int(p[2])),
        ]))
    }

    // ---- accessors -----------------------------------------------------

    /// The `role`th provider of the command (`entity`, `block` or `storage`).
    pub(in crate::game::server_side) fn data_ref(
        &mut self,
        ctx: &Context,
        role: usize,
    ) -> R<DataRef> {
        let prefix = if role == 0 { "target" } else { "source" };
        self.data_ref_named(ctx, role, prefix)
    }
    /// `PistonMovingBlockEntity.saveAdditional`, with full command metadata.
    pub(in crate::game::server_side) fn moving_piston_data(&self, p: [i32; 3]) -> Option<Tag> {
        let pos = sunrise_core::position::BlockPos::new(p[0], p[1], p[2]);
        let (_, piston) = self
            .common
            .block_ticks
            .moving_pistons
            .iter()
            .find(|(at, _)| *at == pos)?;
        Some(self.common.moving_piston_to_nbt(pos, piston))
    }
    pub(in crate::game) fn data_ref_named(
        &mut self,
        ctx: &Context,
        role: usize,
        prefix: &str,
    ) -> R<DataRef> {
        let kind = ctx
            .literals
            .iter()
            .filter(|l| matches!(l.as_str(), "entity" | "block" | "storage"))
            .nth(role)
            .cloned()
            .unwrap_or_default();
        match kind.as_str() {
            "entity" => {
                let Some(Arg::Entity(sel)) = ctx.get(prefix).cloned() else {
                    return Err(err("argument.entity.notfound.entity", &[]));
                };
                let found = self.select(&sel);
                found
                    .first()
                    .map(|t| DataRef::Entity(*t))
                    .ok_or_else(|| err("argument.entity.notfound.entity", &[]))
            }
            "block" => {
                let p = self.block_pos_arg(ctx, &format!("{prefix}Pos"))?;
                if !self
                    .common
                    .world
                    .block_entities
                    .contains_key(&(p[0], p[1], p[2]))
                    && self.comparator_data(p).is_none()
                    && self.common.pot_data(p).is_none()
                    && self.moving_piston_data(p).is_none()
                {
                    return Err(err("commands.data.block.invalid", &[]));
                }
                Ok(DataRef::Block(p))
            }
            _ => match ctx.get(prefix) {
                Some(Arg::Resource(id)) => Ok(DataRef::Storage(id.clone())),
                _ => Err(err("command.failed", &[])),
            },
        }
    }
    pub fn data_of(&mut self, r: &DataRef) -> R<Tag> {
        match r {
            // EntityDataAccessor.getData: Entity.saveWithoutId.
            DataRef::Entity(t) => self
                .in_target_level(*t, |g| g.entity_tag_in_level(*t, false))
                .flatten()
                .ok_or_else(|| err("argument.entity.notfound.entity", &[])),
            DataRef::Block(p) => self
                .block_entity_nbt(*p)
                .ok_or_else(|| err("commands.data.block.invalid", &[])),
            DataRef::Storage(id) => Ok(self.common.score.storage.get(id)),
        }
    }
    /// `BlockEntity.saveWithFullMetadata` of the block entity at `p`: no
    /// chunk-only keepPacked, and the lock and custom name the model does
    /// not hold. `None` where there is no block entity.
    pub fn block_entity_nbt(&self, p: [i32; 3]) -> Option<Tag> {
        // A live moving piston is the level's block entity there (#9617).
        if let Some(tag) = self.moving_piston_data(p) {
            return Some(tag);
        }
        self.full_block_entity_data(p)
    }
    /// `BlockEntity.saveWithFullMetadata` of the stored block entity at `p`,
    /// for [`Self::block_entity_nbt`].
    /// Comparators and decorated pots come from their own views; any other
    /// entry gets what the model keeps outside it: the hopper's
    /// `TransferCooldown` (`HopperBlockEntity.saveAdditional`), the custom
    /// name and the pending loot table.
    pub(in crate::game::server_side) fn full_block_entity_data(&self, p: [i32; 3]) -> Option<Tag> {
        if let Some(tag) = self.comparator_data(p).or_else(|| self.common.pot_data(p)) {
            return Some(tag);
        }
        let key = (p[0], p[1], p[2]);
        let name = self.common.data_block_name(p);
        let e = self.common.world.block_entities.get(&key)?;
        let pending = self
            .common
            .world
            .chunks
            .get(&(key.0 >> 4, key.2 >> 4))
            .and_then(|chunk| chunk.saved.as_ref())
            .and_then(|saved| saved.pending_loot.get(&key));
        let items = self.items();
        let current = save::block_entity_encode::data_with(
            key,
            e,
            &name,
            save::block_entity_encode::DataFields {
                lock: self.common.world.block_lock(key),
                custom_name: self.common.world.block_names.get(&key),
                pending,
                hopper_cooldown: (name == "minecraft:hopper").then(|| self.hopper_cooldown(key)),
            },
            items,
            |row| self.capture_block_entity_payload(key, &name, row, items),
        );
        // Over the NBT it was loaded from, as the chunk save and a loot roll read it.
        Some(
            sunrise_gameplay_common::game::spill_common::live_full_metadata(
                &self.common.world,
                key,
                e,
                &name,
                items,
                current,
            ),
        )
    }
    /// `BlockEntity.loadWithComponents` for the block entity at `key` from its full saved
    /// `tag` (id and position included): false when the tag doesn't decode.
    pub(in crate::game::server_side) fn load_block_entity_data(
        &mut self,
        key: (i32, i32, i32),
        tag: &Tag,
    ) -> bool {
        let state = self.common.world.block(key.0, key.1, key.2);
        let name = &self.common.res.facts().get(state).name;
        let previous = self.common.world.block_entities.get(&key);
        let Some((_, mut new)) = save::decode_block_entity_replacing(tag, name, previous) else {
            return false;
        };
        self.take_loot_table(key, tag, &mut new);
        self.common.load_block_name(key, &new, tag);
        self.common.world.block_entities.insert(key, new);
        self.common.world.set_block_lock(key, tag);
        true
    }
    /// What `BlockDataAccessor.setData` tells the world after a load: `setChanged` and
    /// `sendBlockUpdated(3)`, even when the block state stays the same.
    pub(in crate::game::server_side) fn block_entity_updated(&mut self, key: (i32, i32, i32)) {
        if self.defer_unpublished_block_entity_update(key) {
            return;
        }
        self.mark_block_entity_updated(key);
        let state = self.common.world.block(key.0, key.1, key.2);
        // Redraw what shows the block entity (signs, banners).
        let _ = self.common.world.set(key.0, key.1, key.2, state);
    }
    pub(in crate::game::server_side) fn data_name(&self, r: &DataRef) -> Text {
        match r {
            DataRef::Entity(t) => self.target_name(*t),
            DataRef::Block(p) => Text::literal(format!("{}, {}, {}", p[0], p[1], p[2])),
            DataRef::Storage(id) => Text::literal(id.clone()),
        }
    }
    pub(in crate::game::server_side) fn modified_text(&self, r: &DataRef) -> Text {
        match r {
            DataRef::Entity(_) => {
                Text::translate("commands.data.entity.modified", &[self.data_name(r)])
            }
            DataRef::Block(p) => Text::translate(
                "commands.data.block.modified",
                &[
                    Text::literal(p[0].to_string()),
                    Text::literal(p[1].to_string()),
                    Text::literal(p[2].to_string()),
                ],
            ),
            DataRef::Storage(id) => Text::translate(
                "commands.data.storage.modified",
                &[Text::literal(id.clone())],
            ),
        }
    }
    pub(in crate::game::server_side) fn query_text(&self, r: &DataRef, data: &Tag) -> Text {
        let pretty = snbt::pretty(data);
        match r {
            DataRef::Entity(_) => {
                Text::translate("commands.data.entity.query", &[self.data_name(r), pretty])
            }
            DataRef::Block(p) => Text::translate(
                "commands.data.block.query",
                &[
                    Text::literal(p[0].to_string()),
                    Text::literal(p[1].to_string()),
                    Text::literal(p[2].to_string()),
                    pretty,
                ],
            ),
            DataRef::Storage(id) => Text::translate(
                "commands.data.storage.query",
                &[Text::literal(id.clone()), pretty],
            ),
        }
    }
    pub(in crate::game::server_side) fn get_text(
        &self,
        r: &DataRef,
        path: &NbtPath,
        scale: f64,
        value: i32,
    ) -> Text {
        let p = Text::literal(path.as_str());
        let (s, v) = (
            data_cmd::scale_text(scale),
            Text::literal(value.to_string()),
        );
        match r {
            DataRef::Entity(_) => {
                Text::translate("commands.data.entity.get", &[p, self.data_name(r), s, v])
            }
            DataRef::Block(b) => Text::translate(
                "commands.data.block.get",
                &[
                    p,
                    Text::literal(b[0].to_string()),
                    Text::literal(b[1].to_string()),
                    Text::literal(b[2].to_string()),
                    s,
                    v,
                ],
            ),
            DataRef::Storage(id) => Text::translate(
                "commands.data.storage.get",
                &[p, Text::literal(id.clone()), s, v],
            ),
        }
    }

    // ---- /data -----------------------------------------------------------

    pub(in crate::game::server_side) fn path_arg(&self, ctx: &Context, name: &str) -> R<NbtPath> {
        let Some(Arg::Str(text)) = ctx.get(name) else {
            return Err(err("command.failed", &[]));
        };
        NbtPath::parse(text).map_err(path_error)
    }
    pub(in crate::game::server_side) fn cmd_data(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let target = self.data_ref(ctx, 0)?;
        // An entity in another level is read and written in the level that
        // holds it.
        if let DataRef::Entity(t) = &target {
            let t = *t;
            let target = target.clone();
            return match self.in_target_level(t, |g| g.cmd_data_in(id, ctx, target)) {
                Some(r) => r,
                None => Err(err("argument.entity.notfound.entity", &[])),
            };
        }
        self.cmd_data_in(id, ctx, target)
    }
    /// `DataCommands.mergeData`: literal values keep the modern argument
    /// parser's decoded Tag contract through execution.
    pub fn cmd_data_in(&mut self, id: &str, ctx: &Context, target: DataRef) -> R<i32> {
        match id {
            "data.merge" => {
                let Some(Arg::Str(nbt)) = ctx.get("nbt") else {
                    return Ok(0);
                };
                let nbt = sunrise_command::snbt::parse_fully(nbt)?;
                let old = self.data_of(&target)?;
                if sunrise_game_data::nbt_path::too_deep(&nbt, 0) {
                    return Err(err("arguments.nbtpath.too_deep", &[]));
                }
                let result = data_cmd::merged(&old, &nbt);
                if old == result {
                    return Err(err("commands.data.merge.failed", &[]));
                }
                self.set_data_of(&target, result)?;
                let message = self.modified_text(&target);
                self.feedback(message);
                Ok(1)
            }
            "data.get" => {
                let data = self.data_of(&target)?;
                if ctx.get("path").is_none() {
                    let message = self.query_text(&target, &data);
                    self.system_feedback(message);
                    return Ok(1);
                }
                let path = self.path_arg(ctx, "path")?;
                let tag = data_cmd::single_tag(&path, &data)?;
                if let Some(Arg::Double(scale)) = ctx.get("scale") {
                    let value = data_cmd::scaled_value(&tag, &path, *scale)?;
                    let message = self.get_text(&target, &path, *scale, value);
                    self.system_feedback(message);
                    return Ok(value);
                }
                let value = data_cmd::get_value(&tag, &path)?;
                let message = self.query_text(&target, &tag);
                self.system_feedback(message);
                Ok(value)
            }
            "data.remove" => {
                let path = self.path_arg(ctx, "path")?;
                let mut data = self.data_of(&target)?;
                let count = path.remove(&mut data);
                if count == 0 {
                    return Err(err("commands.data.merge.failed", &[]));
                }
                self.set_data_of(&target, data)?;
                let message = self.modified_text(&target);
                self.feedback(message);
                Ok(count as i32)
            }
            "data.modify" => self.cmd_data_modify(ctx, target),
            _ => Err(err("command.failed", &[])),
        }
    }
    /// A message sent without the feedback gamerule check (`sendSuccess(.., false)`).
    pub(in crate::game::server_side) fn system_feedback(&mut self, message: Text) {
        self.feedback(message);
    }
    /// `DataCommands.manipulateData`: literal sources use the same SNBT
    /// decode contract as command admission; stored sources are unchanged.
    pub(in crate::game::server_side) fn cmd_data_modify(
        &mut self,
        ctx: &Context,
        target: DataRef,
    ) -> R<i32> {
        let target_path = self.path_arg(ctx, "targetPath")?;
        let after_modify: Vec<&String> = ctx
            .literals
            .iter()
            .skip_while(|l| l.as_str() != "modify")
            .collect();
        let op = after_modify
            .iter()
            .find_map(|l| match l.as_str() {
                "prepend" => Some(Op::Prepend),
                "append" => Some(Op::Append),
                "set" => Some(Op::Set),
                "merge" => Some(Op::Merge),
                "insert" => Some(Op::Insert(match ctx.get("index") {
                    Some(Arg::Int(i)) => *i,
                    _ => 0,
                })),
                _ => None,
            })
            .unwrap_or(Op::Set);
        let sources: Vec<Tag> = if ctx.has("value") {
            let Some(Arg::Str(text)) = ctx.get("value") else {
                return Ok(0);
            };
            vec![sunrise_command::snbt::parse_fully(text)?]
        } else if ctx.get("provider").is_some() {
            // `LootContextSources`' compute source: the provider's value in the
            // command's loot context, as a one-tag source.
            let compute = self.compute_context(ctx)?;
            let integer = ctx.literals.iter().any(|l| l == "integer");
            let registry = if integer {
                "minecraft:context_int_provider"
            } else {
                "minecraft:context_float_provider"
            };
            let (_, provider) = self.provider_arg(ctx, registry)?;
            if integer {
                // ContextIntProvider.getInt: an ArithmeticException reads as 0.
                let value = self.provider_int(&provider, &compute).unwrap_or(0);
                vec![Tag::Int(value)]
            } else {
                // ContextFloatProvider.getFloat: a failure or a non-finite
                // value reads as 0.
                let value = self.provider_float(&provider, &compute).unwrap_or(0.0);
                let value = if value.is_finite() { value } else { 0.0 };
                vec![Tag::Float(value.to_bits())]
            }
        } else {
            let source = self.data_ref(ctx, 1)?;
            let data = self.data_of(&source)?;
            let tags = if ctx.get("sourcePath").is_some() {
                self.path_arg(ctx, "sourcePath")?
                    .get(&data)
                    .map_err(path_error)?
            } else {
                vec![data]
            };
            if ctx.has("string") {
                let start = match ctx.get("start") {
                    Some(Arg::Int(i)) => Some(*i),
                    _ => None,
                };
                let end = match ctx.get("end") {
                    Some(Arg::Int(i)) => Some(*i),
                    _ => None,
                };
                let mut out = Vec::new();
                for t in &tags {
                    let text = data_cmd::as_text(t)?;
                    let text = match start {
                        Some(s) => data_cmd::substring(&text, s, end)?,
                        None => text,
                    };
                    out.push(Tag::String(text.as_str().into()));
                }
                out
            } else {
                tags
            }
        };
        let mut data = self.data_of(&target)?;
        let changed = data_cmd::modify(op, &mut data, &target_path, &sources)?;
        if changed == 0 {
            // StorageDataAccessor.getData is the stored compound itself: an
            // edit that changed it and still counted nothing (a wildcard set
            // that clears an array, then rejects the value) stays. An entity's
            // or a block's data is a copy.
            if let DataRef::Storage(id) = &target {
                self.common.score.storage.set_unsaved(id, data);
            }
            return Err(err("commands.data.merge.failed", &[]));
        }
        self.set_data_of(&target, data)?;
        let message = self.modified_text(&target);
        self.feedback(message);
        Ok(changed as i32)
    }

    // ---- /tag ------------------------------------------------------------

    pub(in crate::game::server_side) fn cmd_tag(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let targets = self.entities(ctx, "targets", false)?;
        let name = match ctx.get("name") {
            Some(Arg::Str(n)) => n.clone(),
            _ => String::new(),
        };
        let mut tracker = Tracker::default();
        let holder = |g: &Self, t: Target| Holder {
            name: String::new(),
            text: g.target_name(t),
            display: None,
        };
        // A natural knot is listed but keeps no tags yet: `tag add`/`remove` leave it out.
        let targets = if matches!(id, "tag.add" | "tag.remove") {
            super::without_knots(targets)?
        } else {
            targets
        };
        let mut all: Vec<String> = Vec::new();
        for &t in &targets {
            let mut tags = self.tags_of(t);
            let h = holder(self, t);
            match id {
                "tag.add" => {
                    let added = tags.len() < 1024 && !tags.contains(&name);
                    if added {
                        tags.push(name.clone());
                        self.set_entity_tags(t, &tags);
                    }
                    tracker.track(&h, i32::from(added));
                }
                "tag.remove" => {
                    let before = tags.len();
                    tags.retain(|x| x != &name);
                    let removed = tags.len() != before;
                    if removed {
                        self.set_entity_tags(t, &tags);
                    }
                    tracker.track(&h, i32::from(removed));
                }
                _ => {
                    tracker.track(&h, tags.len() as i32);
                    for tag in tags {
                        if !all.contains(&tag) {
                            all.push(tag);
                        }
                    }
                }
            }
        }
        let num = |n: usize| Text::literal(n.to_string());
        let n = name.clone();
        let (message, total) = match id {
            "tag.add" => tracker.message(
                false,
                Some(err("commands.tag.add.failed", &[])),
                |h, _| {
                    Text::translate(
                        "commands.tag.add.success.single",
                        &[Text::literal(n.clone()), h.text.clone()],
                    )
                },
                |c, _| {
                    Text::translate(
                        "commands.tag.add.success.multiple",
                        &[Text::literal(n.clone()), num(c)],
                    )
                },
            ),
            "tag.remove" => tracker.message(
                false,
                Some(err("commands.tag.remove.failed", &[])),
                |h, _| {
                    Text::translate(
                        "commands.tag.remove.success.single",
                        &[Text::literal(n.clone()), h.text.clone()],
                    )
                },
                |c, _| {
                    Text::translate(
                        "commands.tag.remove.success.multiple",
                        &[Text::literal(n.clone()), num(c)],
                    )
                },
            ),
            _ => {
                let count = all.len();
                let message = if all.is_empty() {
                    tracker
                        .message(
                            false,
                            None,
                            |h, _| {
                                Text::translate(
                                    "commands.tag.list.single.empty",
                                    std::slice::from_ref(&h.text),
                                )
                            },
                            |c, _| Text::translate("commands.tag.list.multiple.empty", &[num(c)]),
                        )?
                        .0
                } else {
                    let list = {
                        let mut sorted = all.clone();
                        sorted.sort();
                        format_list(
                            sorted
                                .into_iter()
                                .map(|s| {
                                    Text::literal(s).with_color(sunrise_game_data::text::GREEN)
                                })
                                .collect(),
                        )
                    };
                    // `TagCommand.listTags` tracks each entity's tag count and
                    // sends with `sendFeedback`'s default element type, NON_ZERO:
                    // the single form names the one entity that has tags.
                    tracker
                        .message(
                            false,
                            None,
                            |h, _| {
                                Text::translate(
                                    "commands.tag.list.single.success",
                                    &[h.text.clone(), num(count), list.clone()],
                                )
                            },
                            |c, _| {
                                Text::translate(
                                    "commands.tag.list.multiple.success",
                                    &[num(c), num(count), list.clone()],
                                )
                            },
                        )?
                        .0
                };
                // `TagCommand.listTags` reports with `sendFeedback(source, false, …)`:
                // the operators are not told, unlike `addTag`/`removeTag` (`true`).
                self.feedback_unbroadcast(message);
                return Ok(count as i32);
            }
        }?;
        self.feedback(message);
        Ok(total)
    }

    // ---- /summon ---------------------------------------------------------

    /// The `summon <entity> <pos> <nbt>` form.
    pub fn summon_with_nbt(&mut self, kind: &str, pos: [f64; 3], nbt: &str) -> R<Option<Target>> {
        let (tag, _) = parse_snbt(nbt).map_err(|e| err("argument.nbt.invalid", &[&e]))?;
        let Tag::Compound(nbt) = tag else {
            return Err(err("argument.nbt.expected.compound", &[]));
        };
        self.summon_from_compound(kind, pos, nbt)
    }

    /// `SummonCommand.createEntity` with explicit NBT, shared by commands and level handles.
    pub(in crate::game) fn summon_from_compound(
        &mut self,
        kind: &str,
        pos: [f64; 3],
        nbt: Compound,
    ) -> R<Option<Target>> {
        if !summon_position_ok(pos) {
            return Err(err("commands.summon.invalidPosition", &[]));
        }
        if matches!(kind, "minecraft:player" | "minecraft:fishing_bobber") {
            return Err(err("entity.not_summonable", &[&kind]));
        }
        let peaceful = self.difficulty_id() == 0;
        if peaceful
            && self
                .common
                .res
                .mob_types()
                .get(kind)
                .is_some_and(|t| !t.allowed_in_peaceful())
        {
            return Err(err("commands.summon.failed.peaceful", &[]));
        }
        let mut tag = Tag::Compound(nbt);
        // SummonCommand.createEntity with an NBT argument passes
        // `finalize = false`: the mob is what its compound says, and the
        // `sunrise:finalize` marker worldgen leaves on a structure mob is not
        // the player's to set.
        strip_finalize_tree(&mut tag);
        set_key(&mut tag, "id", Tag::String(kind.into()));
        // Turtle.readAdditionalSaveData chooses its home before SummonCommand's snapTo.
        // Preserve that load-stage default while the shared loader receives the final Pos.
        if kind == "minecraft:turtle"
            && !matches!(tag.get("home_pos"), Some(Tag::IntArray(home)) if home.len() == 3)
        {
            let home = entity_nbt::position(&tag).map_or([0; 3], |p| {
                [p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32]
            });
            set_key(&mut tag, "home_pos", Tag::IntArray(home.to_vec()));
        }
        if peaceful {
            self.prune_peaceful_summon_passengers(&mut tag);
        }
        place_summoned(&mut tag, pos);
        let Some(tree) = self.stage_entity_tree(tag) else {
            return Ok(None);
        };
        // Check UUIDs only for the feature-eligible tree that will be published.
        if self
            .summon_tree_uuids(tree.tag())
            .into_iter()
            .any(|u| self.is_loaded_uuid_in_level(u))
        {
            return Err(err("commands.summon.failed.uuid", &[]));
        }
        let from = self.common.mobs.len();
        let canceled = self.common.persist.joins_canceled;
        let target = self.spawn_staged_entity_from_nbt(tree);
        self.admit_new_mobs(from);
        // A mod kept the entity out: tryAddFreshEntityWithPassengers is false.
        if self.common.persist.joins_canceled != canceled
            && target.is_none_or(|t| self.target_position(t).is_none())
        {
            return Err(err("commands.summon.failed.uuid", &[]));
        }
        Ok(target)
    }

    /// Checked `EntityType.loadEntityRecursive` drops an ineligible passenger
    /// and its entire subtree before UUID admission. Filter only the summon
    /// input; the shared loader must still restore those entities from saves.
    pub(in crate::game::server_side) fn prune_peaceful_summon_passengers(&self, tag: &mut Tag) {
        let Tag::Compound(compound) = tag else { return };
        if let Some(Tag::List(riders)) = compound.get_mut(&"Passengers".into()) {
            riders.retain_mut(|rider| {
                if let Some(kind) = entity_nbt::kind(rider)
                    && self
                        .common
                        .res
                        .mob_types()
                        .get(&kind)
                        .is_some_and(|t| !t.allowed_in_peaceful())
                {
                    return false;
                }
                self.prune_peaceful_summon_passengers(rider);
                true
            });
        }
    }
}

/// Reserve worldgen's `sunrise:finalize` by stripping it from NBT trees supplied by `/summon`, `LevelRef::summon`, `/data` and `minecraft:entity_data`.
pub(in crate::game::server_side) fn strip_finalize_tree(tag: &mut Tag) {
    let Tag::Compound(c) = tag else { return };
    c.remove(&"sunrise:finalize".into());
    if let Some(Tag::List(passengers)) = c.get_mut(&"Passengers".into()) {
        for passenger in passengers {
            strip_finalize_tree(passenger);
        }
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;
use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::native_hooks::block_entity_save_additional::BlockEntitySaveAdditionalGameExt1 as _;

#[cfg(test)]
pub(in crate::game) mod modern_literal_tests {}
