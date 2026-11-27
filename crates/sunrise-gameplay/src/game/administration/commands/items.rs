//! `/item` (ItemCommands) and `if items`/`if slots`: addressing the item
//! slots of the player, mobs, armor stands, item frames and containers.
use super::{Arg, Context, R, Target, err};
use sunrise_command::{CommandError, args::ItemInput};
use sunrise_core::nbt::Tag;
use sunrise_game_data::text::Text;
use sunrise_items::items::ItemStack;
use sunrise_level::containers::BlockEntity;
use sunrise_sim::scoreboard_cmd::Holder;

pub(in crate::game) mod contents;
pub(in crate::game) mod slot_sources;
use sunrise_gameplay_common::game::commands_common::Component;
pub use sunrise_gameplay_common::game::commands_common::Slot;

pub(in crate::game) struct SlotCandidate {
    pub(in crate::game) slot: Slot,
    pub(in crate::game) filters: Vec<serde_json::Value>,
    pub(in crate::game) limit: usize,
}
/// Whose slots a command addresses.
#[derive(Clone, Debug, PartialEq)]
pub enum Owner {
    Entity(Target),
    Block([i32; 3]),
}

pub(in crate::game) fn mob_slot(name: &str) -> Option<usize> {
    Some(match name {
        "mainhand" => 0,
        "offhand" => 1,
        "feet" => 2,
        "legs" => 3,
        "chest" => 4,
        "head" => 5,
        "body" => 6,
        "saddle" => 7,
        _ => return None,
    })
}

/// The player's slot names and the inventory index each addresses.
pub(in crate::game) fn player_slot_names() -> Vec<(String, usize)> {
    let mut v = Vec::new();
    for i in 0..36 {
        v.push((format!("container.{i}"), i));
    }
    for i in 0..9 {
        v.push((format!("hotbar.{i}"), i));
    }
    for i in 0..27 {
        v.push((format!("inventory.{i}"), 9 + i));
    }
    v
}
/// `name` against a group: `container.*` matches every `container.N`.
pub(in crate::game) fn matches_name(pattern: &str, name: &str) -> bool {
    match pattern.strip_suffix(".*") {
        Some(prefix) => name.starts_with(&format!("{prefix}.")),
        None => pattern == name,
    }
}

impl crate::game::ServerGame<'_> {
    pub(in crate::game) fn owner_of(
        &mut self,
        ctx: &Context,
        prefix: &str,
        many: bool,
    ) -> R<Vec<Owner>> {
        let kind = ctx
            .literals
            .iter()
            .filter(|l| matches!(l.as_str(), "entity" | "block"))
            .nth(usize::from(prefix == "source" && ctx.has("from")))
            .cloned()
            .unwrap_or_default();
        if kind == "block" {
            let p = self.block_pos_arg(ctx, &format!("{prefix}Pos"))?;
            return Ok(vec![Owner::Block(p)]);
        }
        let targets = self.entities(ctx, prefix, false)?;
        let _ = many;
        Ok(targets.into_iter().map(Owner::Entity).collect())
    }
    /// The slots a name (or wildcard group) addresses on an owner.
    pub fn slots_named(&mut self, owner: &Owner, name: &str) -> Vec<Slot> {
        if let Owner::Entity(Target::Guest(conn)) = *owner {
            let slots = self.for_player(Target::Guest(conn), |g| {
                g.slots_named(&Owner::Entity(Target::Player), name)
            });
            return slots
                .unwrap_or_default()
                .into_iter()
                .map(|s| Slot::Guest(conn, Box::new(s)))
                .collect();
        }
        let mut out = Vec::new();
        let mut add = |s: Slot| out.push(s);
        match owner {
            Owner::Block(p) => {
                let size = self.block_container_size(*p).unwrap_or(0);
                for i in 0..size {
                    if matches_name(name, &format!("container.{i}")) {
                        add(Slot::Block(*p, i));
                    }
                }
            }
            Owner::Entity(Target::Player) => {
                for (n, i) in player_slot_names() {
                    if matches_name(name, &n) {
                        add(Slot::Inv(i));
                    }
                }
                for (i, n) in ["armor.head", "armor.chest", "armor.legs", "armor.feet"]
                    .into_iter()
                    .enumerate()
                {
                    if matches_name(name, n) {
                        add(Slot::Armor(i));
                    }
                }
                if matches_name(name, "weapon") || matches_name(name, "weapon.mainhand") {
                    add(Slot::MainHand);
                }
                if matches_name(name, "weapon.offhand") {
                    add(Slot::Offhand);
                }
                for (i, n) in [
                    (sunrise_items::equipment::BODY, "armor.body"),
                    (sunrise_items::equipment::SADDLE, "saddle"),
                ] {
                    if matches_name(name, n) {
                        add(Slot::Equipment(i));
                    }
                }
                for i in 0..27 {
                    if matches_name(name, &format!("enderchest.{i}")) {
                        add(Slot::Ender(i));
                    }
                }
            }
            Owner::Entity(Target::Mob(id)) => {
                for (n, key) in [
                    ("weapon", "mainhand"),
                    ("weapon.mainhand", "mainhand"),
                    ("weapon.offhand", "offhand"),
                    ("armor.head", "head"),
                    ("armor.chest", "chest"),
                    ("armor.legs", "legs"),
                    ("armor.feet", "feet"),
                    ("armor.body", "body"),
                    ("saddle", "saddle"),
                ] {
                    if matches_name(name, n) {
                        add(Slot::Mob(*id, key));
                    }
                }
            }
            // A stand and the dragon hold the shared `Equipment`: every slot.
            Owner::Entity(target @ (Target::Stand(id) | Target::Dragon(id))) => {
                for (i, key) in sunrise_items::equipment::SLOT_NAMES.iter().enumerate() {
                    let aliases: &[&str] = match *key {
                        "mainhand" => &["weapon", "weapon.mainhand"],
                        "offhand" => &["weapon.offhand"],
                        "feet" => &["armor.feet"],
                        "legs" => &["armor.legs"],
                        "chest" => &["armor.chest"],
                        "head" => &["armor.head"],
                        "body" => &["armor.body"],
                        _ => &["saddle"],
                    };
                    if aliases.iter().any(|a| matches_name(name, a)) {
                        add(if matches!(target, Target::Dragon(_)) {
                            Slot::Dragon(*id, i)
                        } else {
                            Slot::Stand(*id, i)
                        });
                    }
                }
            }
            Owner::Entity(Target::Frame(id)) => {
                if matches_name(name, "container.0") || matches_name(name, "contents") {
                    add(Slot::Frame(*id));
                }
            }
            // `ItemEntity.getSlot(0)`: `contents`, `container.0` and `hotbar.0`
            // are all slot 0 (`SlotRanges`), and it holds the stack.
            Owner::Entity(Target::Item(id)) => {
                if matches_name(name, "contents")
                    || matches_name(name, "container.0")
                    || matches_name(name, "hotbar.0")
                {
                    add(Slot::Item(*id));
                }
            }
            // `getChestVehicleSlot` takes any index below `getContainerSize`.
            Owner::Entity(Target::Cart(id)) => {
                let size = self
                    .common
                    .carts
                    .iter()
                    .find(|c| c.id == *id)
                    .map_or(0, |c| c.kind.slots());
                for i in 0..size {
                    if matches_name(name, &format!("container.{i}")) {
                        add(Slot::Cart(*id, i));
                    }
                }
            }
            Owner::Entity(_) => {}
        }
        out
    }
    /// `ItemCommands.getSlotsFromProvider`: evaluate inline sources against
    /// the accessor's live owner. Named ranges retain their existing adapter.
    pub(in crate::game::server_side) fn command_slots(
        &mut self,
        owner: &Owner,
        raw: &str,
    ) -> R<Vec<Slot>> {
        if let Owner::Block(p) = owner
            && self.block_container_size(*p).is_none()
        {
            return Err(err(
                "commands.item.source.not_a_container",
                &[&p[0], &p[1], &p[2]],
            ));
        }
        Ok(match self.resolved_slot_source(raw)? {
            Some(source) => self.inline_slots(owner, &source),
            None => self.slots_named(owner, raw),
        })
    }
    /// `SlotCollection.itemCopies` / `size`: transforms apply to the
    /// collection, before a command's item predicate or item modifier.
    pub(in crate::game::server_side) fn inline_slots(
        &mut self,
        owner: &Owner,
        source: &serde_json::Value,
    ) -> Vec<Slot> {
        if let Some(terms) = source.as_array() {
            return terms
                .iter()
                .flat_map(|s| self.inline_slots(owner, s))
                .collect();
        }
        match source["type"]
            .as_str()
            .unwrap_or_default()
            .trim_start_matches("minecraft:")
        {
            "group" => source["terms"].as_array().map_or_else(Vec::new, |terms| {
                terms
                    .iter()
                    .flat_map(|s| self.inline_slots(owner, s))
                    .collect()
            }),
            "slot_range" => self
                .slot_source_owner(owner, source)
                .map_or_else(Vec::new, |owner| {
                    self.slots_named(&owner, source["slots"].as_str().unwrap_or_default())
                }),
            "contents" => {
                let Some(component) = source["component"].as_str().and_then(Component::named)
                else {
                    return Vec::new();
                };
                let parents = self.inline_slots(owner, &source["slot_source"]);
                parents
                    .into_iter()
                    .flat_map(|p| self.contents_slots(p, component))
                    .collect()
            }
            "limit_slots" => {
                let mut slots = self.inline_slots(owner, &source["slot_source"]);
                slots.truncate(slot_sources::limit(source));
                slots
            }
            "filtered" => {
                let slots = self.inline_slots(owner, &source["slot_source"]);
                slots
                    .into_iter()
                    .filter(|slot| self.slot_filter(slot, &source["item_filter"]))
                    .collect()
            }
            _ => Vec::new(),
        }
    }
    pub(in crate::game::server_side) fn slot_filter(
        &mut self,
        slot: &Slot,
        filter: &serde_json::Value,
    ) -> bool {
        let stack = self
            .slot_get(slot)
            .unwrap_or_else(|| ItemStack::new("minecraft:air", 0));
        self.loot_item_filter(filter, &stack)
    }
    /// `SlotCollection.modifySlots` / `replaceSlotItems`: filters inspect the
    /// live stack when visited. Limits share the command's TrackingSelector
    /// count across owners and group terms, and count selected empty slots
    /// only for replacement (not modification).
    pub(in crate::game::server_side) fn command_slot_candidates(
        &mut self,
        owner: &Owner,
        raw: &str,
    ) -> R<Vec<SlotCandidate>> {
        if let Owner::Block(p) = owner
            && self.block_container_size(*p).is_none()
        {
            return Err(err(
                "commands.item.target.not_a_container",
                &[&p[0], &p[1], &p[2]],
            ));
        }
        if let Some(source) = self.resolved_slot_source(raw)? {
            let mut out = Vec::new();
            self.slot_candidates(owner, &source, &[], usize::MAX, &mut out);
            Ok(out)
        } else {
            Ok(self
                .slots_named(owner, raw)
                .into_iter()
                .map(|slot| SlotCandidate {
                    slot,
                    filters: Vec::new(),
                    limit: usize::MAX,
                })
                .collect())
        }
    }
    /// `CompositeSlotSource.provide` and `TransformedSlotSource.provide`:
    /// carry each leaf's filters and selection bound into live writes.
    pub(in crate::game::server_side) fn slot_candidates(
        &mut self,
        owner: &Owner,
        source: &serde_json::Value,
        filters: &[serde_json::Value],
        limit: usize,
        out: &mut Vec<SlotCandidate>,
    ) {
        if let Some(terms) = source.as_array() {
            for term in terms {
                self.slot_candidates(owner, term, filters, limit, out);
            }
            return;
        }
        match source["type"]
            .as_str()
            .unwrap_or_default()
            .trim_start_matches("minecraft:")
        {
            "group" => {
                if let Some(terms) = source["terms"].as_array() {
                    for term in terms {
                        self.slot_candidates(owner, term, filters, limit, out);
                    }
                }
            }
            "slot_range" => {
                let Some(owner) = self.slot_source_owner(owner, source) else {
                    return;
                };
                for slot in self.slots_named(&owner, source["slots"].as_str().unwrap_or_default()) {
                    out.push(SlotCandidate {
                        slot,
                        filters: filters.to_vec(),
                        limit,
                    });
                }
            }
            "contents" => {
                if let Some(component) = source["component"].as_str().and_then(Component::named) {
                    for parent in self.inline_slots(owner, &source["slot_source"]) {
                        out.push(SlotCandidate {
                            slot: Slot::Contents(Box::new(parent), component),
                            filters: filters.to_vec(),
                            limit,
                        });
                    }
                }
            }
            "filtered" => {
                let mut filters = filters.to_vec();
                filters.push(source["item_filter"].clone());
                self.slot_candidates(owner, &source["slot_source"], &filters, limit, out);
            }
            "limit_slots" => self.slot_candidates(
                owner,
                &source["slot_source"],
                filters,
                limit.min(slot_sources::limit(source)),
                out,
            ),
            _ => {}
        }
    }
    /// `SlotSelector.TrackingSelector.trySelectSlot`: count only slots that
    /// pass the predicate, before attempting the setter.
    pub(in crate::game) fn select_slot(
        &mut self,
        candidate: &SlotCandidate,
        selected: &mut usize,
        nonempty: bool,
    ) -> bool {
        if *selected >= candidate.limit
            || (nonempty && self.slot_get(&candidate.slot).is_none())
            || !candidate
                .filters
                .iter()
                .all(|f| self.slot_filter(&candidate.slot, f))
        {
            return false;
        }
        *selected += 1;
        true
    }
    pub(in crate::game::server_side) fn slot_accepts(
        &self,
        s: &Slot,
        stack: Option<&ItemStack>,
    ) -> bool {
        let Slot::Block(p, i) = s else {
            return true;
        };
        let air = ItemStack {
            item: "minecraft:air".into(),
            count: 0,
            damage: 0,
            extra: None,
        };
        self.block_can_place(*p, *i, stack.unwrap_or(&air))
    }
    pub(in crate::game::server_side) fn block_container_size(&self, p: [i32; 3]) -> Option<usize> {
        let size = match self.common.world.block_entities.get(&(p[0], p[1], p[2]))? {
            BlockEntity::Mod(_) => self
                .common
                .world
                .block_entities
                .mod_container(self.common.world.block(p[0], p[1], p[2]).0)
                .map_or(0, |c| c.slots),
            entity => entity.slots().len(),
        };
        (size > 0).then_some(size)
    }
    /// `LivingEntity.createEquipmentSlotAccess`'s predicate for the slots of a
    /// living entity that are not the head or a hand: an empty stack, or
    /// one whose `getEquipmentSlotForItem` is this very slot. A stand's
    /// `canUseSlot` refuses body, saddle and the slots its `DisabledSlots`
    /// lock, so such an item is MAINHAND's. Every other slot takes anything.
    pub(in crate::game::server_side) fn equipment_slot_accepts(
        &self,
        slot: &Slot,
        stack: &Option<ItemStack>,
    ) -> bool {
        use sunrise_items::equipment::{BODY, CHEST, FEET, LEGS, MAINHAND, SADDLE};
        let (index, stand) = match slot {
            Slot::Stand(id, i) => (*i, self.common.armor_stands.iter().find(|a| a.id == *id)),
            Slot::Dragon(_, i) => (*i, None),
            Slot::Guest(_, inner) => return self.equipment_slot_accepts(inner, stack),
            Slot::Armor(i @ 1..=3) => ([CHEST, LEGS, FEET][*i - 1], None),
            Slot::Mob(_, key) => match *key {
                "chest" => (CHEST, None),
                "body" => (BODY, None),
                "saddle" => (SADDLE, None),
                "legs" => (LEGS, None),
                "feet" => (FEET, None),
                _ => return true,
            },
            _ => return true,
        };
        let Some(stack) = stack else { return true };
        if ![FEET, LEGS, CHEST, BODY, SADDLE].contains(&index) {
            return true;
        }
        let worn = self.equippable_slot_sound(stack).map(|(s, _)| s as usize);
        let usable = |s: usize| match (&stand, slot) {
            (Some(a), _) => s != BODY && s != SADDLE && !a.is_disabled(s),
            (None, Slot::Stand(..)) => false,
            _ => true,
        };
        // SulfurCube.getEquipmentSlotForItem: swallowable items occupy the body slot.
        if let Slot::Mob(id, "body") = slot
            && self
                .common
                .mobs
                .iter()
                .find(|m| m.id == *id)
                .is_some_and(|m| {
                    m.species() == sunrise_sim::mobs::Species::SulfurCube
                        && sunrise_sim::mobs::sulfur_cube::swallowable(m, &stack.item)
                })
        {
            return true;
        }
        worn.filter(|s| usable(*s)).unwrap_or(MAINHAND) == index
    }
    /// `SlotAccess.set` of a command: `slot_set` when the slot's predicate
    /// takes the stack. A mod's `set_equipment` calls `slot_set` itself and is
    /// not asked.
    pub(in crate::game::server_side) fn slot_set_by_command(
        &mut self,
        slot: &Slot,
        stack: Option<ItemStack>,
    ) -> bool {
        self.equipment_slot_accepts(slot, &stack) && self.slot_set(slot, stack)
    }
    /// Put a stack (or nothing) in a slot; false when the slot cannot take it.
    pub fn slot_set(&mut self, s: &Slot, stack: Option<ItemStack>) -> bool {
        self.slot_set_with_effects(s, stack, true)
    }

    pub(in crate::game::server_side) fn owner_holder(&self, o: &Owner) -> Holder {
        match o {
            Owner::Entity(t) => Holder {
                name: String::new(),
                text: self.target_name(*t),
                display: None,
            },
            Owner::Block(p) => Holder {
                name: String::new(),
                text: Text::literal(format!("{}, {}, {}", p[0], p[1], p[2])),
                display: None,
            },
        }
    }
    pub(in crate::game::server_side) fn stack_name(&self, s: &ItemStack) -> Text {
        self.stack_display_name(s)
    }
    pub(in crate::game::server_side) fn item_from_input(
        &self,
        ctx: &Context,
    ) -> R<Option<ItemStack>> {
        let Some(Arg::Item(input)) = ctx.get("item") else {
            return Ok(None);
        };
        let count = Self::int(ctx, "count").unwrap_or(1).max(1) as u32;
        self.command_item_stack(input, count).map(Some)
    }
    /// `ItemInput.createItemStack`, shared by vanilla commands and native arguments.
    pub(in crate::game) fn command_item_stack(
        &self,
        ItemInput { id, components, .. }: &ItemInput,
        count: u32,
    ) -> R<ItemStack> {
        let (damage, extra) = match components
            .as_deref()
            .map(sunrise_items::item_data::command_components)
        {
            Some(Ok(tag)) => sunrise_items::item_data::decode_components(Some(&tag)),
            Some(Err(e)) => return Err(CommandError::new(Text::literal(e))),
            None => (0, None),
        };
        let stack = ItemStack {
            item: id.as_str().into(),
            count,
            damage,
            extra,
        };
        // ItemInput.createItemStack: a count above the stack's max (its
        // max_stack_size component, else the item's) is refused.
        let max_stack = self.stack_max_size(&stack);
        if count > max_stack {
            return Err(err("arguments.item.overstacked", &[id, &max_stack]));
        }
        // ItemInput.createItemStack validates effective components after count.
        // Materialization currently discards removals, so inspect the validated
        // command patch itself without changing the stack or its saved patch.
        let components = self.item_validation_components(id, components.as_deref())?;
        if components.get(&"minecraft:max_damage".to_owned()).is_some()
            && sunrise_world::datafix::dynamic::as_int(
                components.get(&"minecraft:max_stack_size".to_owned()),
                1,
            ) > 1
        {
            return Err(err(
                "arguments.item.malformed",
                &[&"Item cannot be both damageable and stackable"],
            ));
        }
        Ok(stack)
    }
    /// ItemInput.createItemStack: read-only view of the two prototype components used by this command
    /// validation, with all typed patch operations (including removals).
    pub fn item_validation_components(
        &self,
        id: &str,
        components: Option<&str>,
    ) -> R<sunrise_world::component_patch::ComponentMap<String, Tag>> {
        use sunrise_command::{Reader, args::read_identifier, snbt::parse_tag_argument};
        use sunrise_world::component_patch::{ComponentMap, Patch, Prototype};
        let mut prototype = Vec::new();
        if let Some(item) = self.items().get(id) {
            for (key, value) in [
                ("minecraft:max_damage", item.max_damage),
                ("minecraft:max_stack_size", item.max_stack),
            ] {
                if item.components.iter().any(|component| component == key) {
                    prototype.push((key.to_owned(), Tag::Int(value as i32)));
                }
            }
        }
        let mut patch = Vec::new();
        if let Some(components) = components {
            // ItemInput has already validated IDs, values and duplicates. Reuse
            // its token/value parsers; do not derive removals from saved output.
            let mut reader = Reader::new(components);
            reader.expect('[')?;
            reader.skip_whitespace();
            while reader.can_read() && reader.peek() != ']' {
                let removed = reader.peek() == '!';
                if removed {
                    reader.skip();
                }
                let key = read_identifier(&mut reader)?;
                reader.skip_whitespace();
                let value = if removed {
                    None
                } else {
                    reader.expect('=')?;
                    reader.skip_whitespace();
                    Some(parse_tag_argument(&mut reader)?)
                };
                patch.push((key, value));
                reader.skip_whitespace();
                if reader.peek() == ']' {
                    break;
                }
                reader.expect(',')?;
                reader.skip_whitespace();
            }
            reader.expect(']')?;
        }
        Ok(ComponentMap::from_patch(
            Prototype::new(prototype),
            &Patch::new(patch),
        ))
    }
    pub(in crate::game) fn parse_inline_item_modifier(raw: &str) -> R<serde_json::Value> {
        let (tag, _) =
            sunrise_core::nbt::parse_snbt(raw).map_err(|e| err("argument.nbt.invalid", &[&e]))?;
        let value = sunrise_game_data::scoreboard::tag_to_json(&tag);
        sunrise_world::loot::validate_item_modifier(&value)
            .map_err(|e| err("argument.nbt.invalid", &[&e]))?;
        Ok(value)
    }
    /// `if items` and `if slots`: how many items match / how many slots.
    pub(in crate::game::server_side) fn condition_items(&mut self, c: &Context) -> R<(bool, i32)> {
        let owners = self.owner_of(c, "source", true)?;
        let Some(Arg::Str(slot_name)) = c.get("slots").cloned() else {
            return Ok((false, 0));
        };
        let predicate = match c.get("item_predicate") {
            Some(Arg::Item(p)) => Some(p.clone()),
            _ => None,
        };
        let mut count = 0;
        for o in owners {
            if let Owner::Block(p) = o
                && self.block_container_size(p).is_none()
            {
                return Err(err(
                    "commands.item.source.not_a_container",
                    &[&p[0], &p[1], &p[2]],
                ));
            }
            let slots = self.command_slots(&o, &slot_name)?;
            if predicate.is_none() {
                count += slots.len() as i32;
                continue;
            }
            for s in slots {
                if let Some(stack) = self.slot_get(&s)
                    && predicate
                        .as_ref()
                        .is_some_and(|p| self.stack_matches(p, &stack))
                {
                    count += stack.count as i32;
                }
            }
        }
        Ok((count > 0, count))
    }
}

/// Edit a stack's components as their saved compound.
pub(in crate::game) fn edit_components(
    stack: &mut ItemStack,
    f: impl FnOnce(&mut sunrise_core::nbt::Compound),
) {
    let mut c = match sunrise_items::item_data::component_snapshot(stack) {
        Some(Tag::Compound(c)) => c,
        _ => sunrise_core::nbt::Compound::new(),
    };
    f(&mut c);
    let tag = Tag::Compound(c);
    let (damage, extra) = sunrise_items::item_data::decode_components(Some(&tag));
    stack.damage = damage;
    stack.extra = extra;
    stack.tidy();
}
/// `SetCustomDataFunction.run`: `CustomData.update(stack, tag -> tag.merge(value))`.
pub(in crate::game) fn set_custom_data(stack: &mut ItemStack, value: &Tag) {
    edit_components(stack, |c| {
        let mut data = match c.get(&"minecraft:custom_data".into()) {
            Some(t @ Tag::Compound(_)) => sunrise_core::nbt::copy_tag(t),
            _ => Tag::Compound(Default::default()),
        };
        merge_compound(&mut data, value);
        // Vanilla CustomData.update removes the component when the result is empty.
        if matches!(&data, Tag::Compound(m) if m.is_empty()) {
            c.remove(&"minecraft:custom_data".into());
        } else {
            c.insert("minecraft:custom_data".into(), data);
        }
    });
}
/// `CompoundTag.merge`: nested compounds merge, other values replace.
pub(in crate::game) fn merge_compound(into: &mut Tag, from: &Tag) {
    let (Tag::Compound(a), Tag::Compound(b)) = (into, from) else {
        return;
    };
    for (k, v) in b {
        match (a.get_mut(k), v) {
            (Some(existing @ Tag::Compound(_)), Tag::Compound(_)) => merge_compound(existing, v),
            _ => {
                a.insert(k.clone(), sunrise_core::nbt::copy_tag(v));
            }
        }
    }
}
/// `ListOperation.apply` (`mode`: replace_all, replace_section, insert,
/// append) with a 256-entry limit (`SetLoreFunction`'s). An append, insert or section
/// replacement that cannot be done, an offset past the end or a result over the limit, gives
/// the old list back unchanged, as vanilla does (it logs an error). `replace_all` is the new
/// list as it is in vanilla, whose `ItemLore` then refuses more than 256 lines; here it is cut.
pub(in crate::game) fn list_operation(
    m: &serde_json::Value,
    old: Vec<Tag>,
    new: Vec<Tag>,
) -> Vec<Tag> {
    const LIMIT: usize = 256;
    let offset = m["offset"].as_u64().unwrap_or(0) as usize;
    match m["mode"].as_str().unwrap_or("") {
        "replace_all" => {
            let mut out = new;
            out.truncate(LIMIT);
            out
        }
        "insert" => {
            if offset > old.len() || old.len() + new.len() > LIMIT {
                return old;
            }
            let mut out = old[..offset].to_vec();
            out.extend(new);
            out.extend_from_slice(&old[offset..]);
            out
        }
        "replace_section" => {
            if offset > old.len() {
                return old;
            }
            let size = m["size"].as_u64().map_or(new.len(), |s| s as usize);
            let mut out = old[..offset].to_vec();
            out.extend(new);
            let end = offset.saturating_add(size);
            if end < old.len() {
                out.extend_from_slice(&old[end..]);
            }
            if out.len() > LIMIT {
                return old;
            }
            out
        }
        _ => {
            if old.len() + new.len() > LIMIT {
                return old;
            }
            let mut out = old;
            out.extend(new);
            out
        }
    }
}

/// `ListOperation.Append/Insert/ReplaceSection.apply` with `/item modify`'s lore limit of 256
/// (`SetLoreFunction` passes 256): an operation that cannot be done logs an error in vanilla and
/// gives the old list back unchanged. The expected lists are worked out from the vanilla bytecode
/// (see the pull request), not from the code under test.
#[cfg(test)]
pub(in crate::game) mod list_operation_tests {
    use super::*;
    use serde_json::json;

    fn ints(range: std::ops::Range<i32>) -> Vec<Tag> {
        range.map(Tag::Int).collect()
    }

    #[test]
    fn an_append_that_would_pass_256_entries_changes_nothing() {
        let old = ints(0..250);
        let m = json!({"mode": "append"});
        // 250 + 7 = 257 is over the limit: the old list, whole.
        assert_eq!(list_operation(&m, old.clone(), ints(1000..1007)), old);
        // 250 + 6 = 256 is exactly the limit: all of it.
        let mut full = old.clone();
        full.extend(ints(1000..1006));
        assert_eq!(list_operation(&m, old, ints(1000..1006)), full);
    }

    #[test]
    fn an_insert_past_the_end_or_over_the_limit_changes_nothing() {
        let old = ints(0..3);
        let at = |offset| json!({"mode": "insert", "offset": offset});
        // Offset 4 is past the end of three entries.
        assert_eq!(list_operation(&at(4), old.clone(), ints(9..10)), old);
        // Offset 3 is the end: an insert there is an append.
        assert_eq!(
            list_operation(&at(3), old.clone(), ints(9..11)),
            [0, 1, 2, 9, 10].map(Tag::Int)
        );
        // Offset 1 in the middle.
        assert_eq!(
            list_operation(&at(1), old, ints(9..11)),
            [0, 9, 10, 1, 2].map(Tag::Int)
        );
        // 255 + 2 = 257 is over the limit, at any offset.
        let big = ints(0..255);
        assert_eq!(list_operation(&at(0), big.clone(), ints(9..11)), big);
    }

    #[test]
    fn a_section_replaced_past_the_end_or_over_the_limit_changes_nothing() {
        let old = ints(0..4);
        let section = |offset, size: Option<u64>| {
            let mut m = json!({"mode": "replace_section", "offset": offset});
            if let Some(size) = size {
                m["size"] = json!(size);
            }
            m
        };
        // Offset 5 is past the end of four entries.
        assert_eq!(
            list_operation(&section(5, None), old.clone(), ints(9..10)),
            old
        );
        // Offset 4 is the end: nothing to replace, the new entries go after.
        assert_eq!(
            list_operation(&section(4, None), old.clone(), ints(9..10)),
            [0, 1, 2, 3, 9].map(Tag::Int)
        );
        // Entries 1 and 2 (size 2) become one entry.
        assert_eq!(
            list_operation(&section(1, Some(2)), old.clone(), ints(9..10)),
            [0, 9, 3].map(Tag::Int)
        );
        // No size: as many entries as the new ones.
        assert_eq!(
            list_operation(&section(0, None), ints(0..3), ints(7..9)),
            [7, 8, 2].map(Tag::Int)
        );
        // 256 entries, one replaced by two: 257 is over the limit.
        let full = ints(0..256);
        assert_eq!(
            list_operation(&section(0, Some(1)), full.clone(), ints(9..11)),
            full
        );
    }

    #[test]
    fn replacing_everything_gives_the_new_list() {
        let m = json!({"mode": "replace_all"});
        assert_eq!(
            list_operation(&m, ints(0..3), ints(7..9)),
            [7, 8].map(Tag::Int)
        );
    }
}

#[cfg(test)]
pub(in crate::game) mod registered_slot_source_read_tests {}

#[cfg(any(test, feature = "test-support"))]
pub mod registered_slot_source_preservation_tests {
    #[allow(unused_imports)]
#[cfg(any())]
pub(in crate::game) use crate::game::server_side::commands_server::items::registered_slot_source_preservation_tests::scene;

    pub fn count(game: &mut crate::game::ServerGame<'_>, expected: i32) {
        assert!(game.run_command("execute store result score #r slotprobe run execute if slots block 0 100 0 parity:first_three"));
        assert_eq!(
            game.score.board.score("#r", "slotprobe").unwrap().value,
            expected
        );
    }
}

#[cfg(test)]
pub(in crate::game) mod set_custom_data_tests {
    use super::*;

    #[test]
    fn merging_an_empty_tag_into_a_stack_without_custom_data_adds_no_component() {
        let mut stack = ItemStack::new("minecraft:stick", 1);
        set_custom_data(&mut stack, &Tag::Compound(Default::default()));
        let saved = sunrise_items::item_data::component_snapshot(&stack);
        assert!(saved.is_none_or(|c| c.get("minecraft:custom_data").is_none()));
    }
}

#[cfg(test)]
pub(in crate::game) mod transient_modifier_tests {

    #[test]
    fn inline_command_modifier_refuses_whole_sequence_before_execution() {
        let error = crate::game::ServerGame::parse_inline_item_modifier(
            r#"[{type:"minecraft:set_count",count:2},{type:"minecraft:set_components",components:{"!minecraft:creative_slot_lock":{}}}]"#,
        ).unwrap_err();
        let message = format!("{error:?}");
        assert!(message.contains("minecraft:creative_slot_lock"));
        assert!(message.contains("not a persistent component"));
        let accepted = crate::game::ServerGame::parse_inline_item_modifier(
            r#"[{type:"minecraft:copy_components",source:"block_entity",include:["minecraft:additional_trade_cost"]},{type:"minecraft:set_custom_data",tag:{components:{"minecraft:map_post_processing":"opaque"}}}]"#,
        ).unwrap();
        assert_eq!(accepted.as_array().unwrap().len(), 2);
        assert_eq!(
            accepted[1]["tag"]["components"]["minecraft:map_post_processing"],
            "opaque"
        );
    }
}

#[cfg(test)]
pub(in crate::game) mod item_entity_contents_tests;

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt1 as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;
