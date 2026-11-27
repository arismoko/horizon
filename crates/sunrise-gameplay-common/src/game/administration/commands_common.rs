//! Common command types and data-only helpers shared by server and client modules.
pub use super::chat_common::PLAYER_NAME;
pub use sunrise_command::CommandError;
pub use sunrise_core::{
    geometry::Vec3,
    nbt::Tag,
    version::{BUILD_TIME, DATA_VERSION, STABLE, VANILLA_VERSION, VERSION_ID},
};
pub use sunrise_game_data::text::{self, Style, Text};
pub use sunrise_items::items::ItemStack;

pub use sunrise_level::containers::BlockEntity;
pub use sunrise_sim::{lan::guest::Guest, save};

pub type R<T> = Result<T, CommandError>;

/// An entity a selector found.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    /// The player in place: the host's, or the guest whose turn it is.
    Player,
    /// Another player of a hosted world or server: a guest by connection.
    Guest(u32),
    Mob(u32),
    Item(u32),
    /// An experience orb, by its entity id.
    Orb(u32),
    /// A projectile, by its entity id.
    Projectile(u32),
    Stand(u32),
    Frame(u32),
    Painting(u32),
    Boat(u32),
    Cart(u32),
    /// An entity of a type Sunrise does not simulate (markers, displays, interactions...).
    Other(u32),
    /// Active evoker fangs, including command-loaded ones.
    Fangs(u32),
    /// An End crystal, by its entity id (`minecraft:end_crystal`).
    Crystal(u32),
    /// The Ender Dragon, by its entity id (`minecraft:ender_dragon`).
    Dragon(u32),
    /// One of the dragon's eight `EnderDragonPart`s, by its own entity id.
    DragonPart(u32),
    /// A primed TNT, by its entity id (`minecraft:tnt`).
    Tnt(u32),
    /// A cushion, by its entity id (`minecraft:cushion`).
    Cushion(u32),
    /// A falling block, by its entity id (`minecraft:falling_block`).
    Falling(u32),
    /// A firework rocket, by its entity id (`minecraft:firework_rocket`).
    Rocket(u32),
    /// A thrown trident, by its entity id (`minecraft:trident`).
    Trident(u32),
    /// An ominous item spawner, by its entity id (`minecraft:ominous_item_spawner`).
    Spawner(u32),
    /// A simulated lingering, creeper or dragon area-effect cloud.
    Cloud(u32),
    /// A live lightning bolt (`minecraft:lightning_bolt`).
    Bolt(u32),
    /// A general entity row, by its entity id (#792).
    General(u32),
    /// A cast fishing hook (`minecraft:fishing_bobber`), which lives with its caster.
    Hook(u32),
    /// The fence knot a lead tie made, by entity id.
    Knot(u32),
}

impl Target {
    /// A player, in place or not (`instanceof ServerPlayer`).
    pub fn is_player(self) -> bool {
        matches!(self, Target::Player | Target::Guest(_))
    }

    /// A target with a `LivingEntity` body.
    pub fn is_living(self) -> bool {
        self.is_player() || matches!(self, Self::Mob(_) | Self::Stand(_) | Self::Dragon(_))
    }
}

/// The entity id a summoned stack's root is known by.
pub fn target_id(t: Target) -> Option<u32> {
    match t {
        Target::Mob(id)
        | Target::Item(id)
        | Target::Orb(id)
        | Target::Projectile(id)
        | Target::Stand(id)
        | Target::Frame(id)
        | Target::Painting(id)
        | Target::Boat(id)
        | Target::Cart(id)
        | Target::Other(id)
        | Target::Fangs(id)
        | Target::Crystal(id)
        | Target::Dragon(id)
        | Target::DragonPart(id)
        | Target::Tnt(id)
        | Target::Cushion(id)
        | Target::Falling(id)
        | Target::Rocket(id)
        | Target::Trident(id)
        | Target::Spawner(id)
        | Target::Cloud(id)
        | Target::Bolt(id)
        | Target::Hook(id)
        | Target::Knot(id)
        | Target::General(id) => Some(id),
        Target::Player | Target::Guest(_) => None,
    }
}

/// `Entity.setXRot`: wrap before clamping the pitch.
pub fn entity_pitch(pitch: f32) -> f32 {
    (pitch % 360.0).clamp(-90.0, 90.0)
}

/// `PositionMoveRotation.calculateAbsolute` and `TeleportCommand.performTeleport`.
pub fn teleport_velocity(velocity: Vec3, relative: [bool; 3], fall_flying: bool) -> Vec3 {
    Vec3::new(
        if relative[0] { velocity.x } else { 0.0 },
        if relative[1] && fall_flying {
            velocity.y
        } else {
            0.0
        },
        if relative[2] { velocity.z } else { 0.0 },
    )
}

/// An entity of a type Sunrise does not simulate: raw compound lives in `persist.raw`.
#[derive(Clone, Debug)]
pub struct OtherEntity {
    /// Entity-owned registration metadata; content clones are detached.
    pub attachment_lifetime: sunrise_mod_runtime::attachments::AttachmentLifetime,
    pub join: sunrise_sim::join_seq::JoinSeq,
    pub id: u32,
    pub kind: String,
    pub position: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub display: Option<std::sync::Arc<sunrise_sim::text_display_data::resident::DisplayState>>,
    /// A `minecraft:interaction` saved settings; None for every other kind.
    pub interaction: Option<super::interactions_common::InteractionState>,
    /// Only explicitly loaded leash knots own this BlockAttachedEntity check counter.
    pub knot_check: Option<u8>,
    /// A summoned or loaded `minecraft:eye_of_ender`'s flight; None for every other kind.
    pub eye: Option<super::end_state::SummonedEye>,
    /// What `entity_size` answered at its last size refresh (`EntityEvent.Size`).
    pub size_answer: Option<sunrise_sim::pose::SizeAnswer>,
}

sunrise_sim::impl_joined!(OtherEntity);
sunrise_mod_runtime::impl_attachment_lifetime!(OtherEntity);

impl OtherEntity {
    /// Physical query bounds, not display rendering/culling dimensions.
    pub fn display_marker_or_interaction_bounds(&self) -> Option<sunrise_core::geometry::Aabb> {
        let dimensions = self.vanilla_dimensions()?;
        let vanilla = if let Some(settings) = self.interaction {
            let mut interaction = crate::game::interactions_common::RemoteInteraction::new([
                self.position.x,
                self.position.y,
                self.position.z,
            ]);
            interaction.width = settings.width;
            interaction.height = settings.height;
            interaction.response = settings.response;
            interaction.bounds()
        } else {
            // Display and Marker have zero-sized physical dimensions.
            sunrise_core::geometry::Aabb::new(self.position, self.position)
        };
        Some(sunrise_sim::pose::SizeAnswer::sized_box(
            self.size_answer,
            dimensions,
            self.position,
            vanilla,
        ))
    }

    /// `Entity.getDimensions(pose)` of a display, a marker or an interaction this list holds,
    /// the kinds whose box follows their size here: an interaction's
    /// `EntityDimensions.scalable(width, height)`, none for a display or a marker. None for
    /// every other kind the list keeps without simulating it (a leash knot, an eye of ender),
    /// which isn't asked.
    pub fn vanilla_dimensions(&self) -> Option<[f32; 3]> {
        if !matches!(
            self.kind.as_str(),
            "minecraft:block_display"
                | "minecraft:item_display"
                | "minecraft:text_display"
                | "minecraft:marker"
                | "minecraft:interaction"
        ) {
            return None;
        }
        Some(match self.interaction {
            Some(settings) => [settings.width, settings.height, settings.height * 0.85],
            None => [0.0; 3],
        })
    }
}

pub fn err(key: &str, args: &[&dyn std::fmt::Display]) -> CommandError {
    CommandError::tr(key, args)
}

/// Format and validate a command failure as the sink sees it, including any timestamp.
pub fn formatted_command_failure(message: Text, timestamp: Option<Text>) -> Text {
    use serde_json::Value;
    use sunrise_core::nbt;

    let format = |message| {
        let failure = Text::empty().append(message).with_color(text::RED);
        match &timestamp {
            Some(prefix) => prefix.clone().append(failure),
            None => failure,
        }
    };
    let output = format(message);
    let serializes = || {
        let component = output.component_view();
        let mut limits = nbt::Limits::default();
        // Check depth before recursing into the component converters.
        let mut pending = vec![(component.as_ref(), 0)];
        while let Some((node, depth)) = pending.pop() {
            match node {
                Value::Object(_) | Value::Array(_) if depth >= limits.max_depth => return false,
                Value::Object(fields) => pending.extend(fields.values().map(|v| (v, depth + 1))),
                Value::Array(values) => pending.extend(values.iter().map(|v| (v, depth + 1))),
                _ => {}
            }
        }
        let tag = if timestamp.is_some() {
            sunrise_game_data::scoreboard::component_tag(&component)
        } else {
            sunrise_sim::lan::data::component_nbt(&component)
        };
        if timestamp.is_some() {
            // The saved component sits under chunk root, block_entities/Entities, and entry.
            limits.max_depth = nbt::Limits::saved().max_depth - 3;
        }
        // Use the strict writer so overlong strings and deep wrappers are refused.
        nbt::write_network(&tag, limits).is_ok()
    };
    if serializes() {
        output
    } else {
        format(Text::tr("command.failed", &[]))
    }
}

/// Exclude natural fence knots from targets a command moves or kills (#13036).
pub fn without_knots(targets: Vec<Target>) -> R<Vec<Target>> {
    let found = !targets.is_empty();
    let kept: Vec<Target> = targets
        .into_iter()
        .filter(|t| !matches!(t, Target::Knot(_)))
        .collect();
    if found && kept.is_empty() {
        return Err(err("argument.entity.notfound.entity", &[]));
    }
    Ok(kept)
}

/// `EntityType.getDescriptionId`: `entity.<namespace>.<path>`.
pub fn entity_name_key(id: &str) -> String {
    let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", id));
    format!("entity.{namespace}.{}", path.replace('/', "."))
}

pub fn texts(args: &[Text]) -> Vec<Text> {
    args.to_vec()
}

/// String.format(Locale.ROOT, "%.<decimals>f"): half-up decimal rounding.
pub fn percent_f(v: f64, decimals: usize) -> String {
    sunrise_core::java_number::format_fixed(v, decimals)
}

/// String.format(Locale.ROOT, "%f").
pub fn format_double(v: f64) -> String {
    percent_f(v, 6)
}

/// `EntitySelectorParser.createRotationPredicate`: wrapped angle range check.
pub fn rotation_in((lo, hi): (Option<f64>, Option<f64>), angle: f32) -> bool {
    let min = wrap_degrees(lo.unwrap_or(0.0) as f32);
    let max = wrap_degrees(hi.unwrap_or(359.0) as f32);
    let angle = wrap_degrees(angle);
    if min > max {
        angle >= min || angle <= max
    } else {
        angle >= min && angle <= max
    }
}

/// `Mth.wrapDegrees`.
pub fn wrap_degrees(angle: f32) -> f32 {
    let wrapped = angle % 360.0;
    if wrapped >= 180.0 {
        wrapped - 360.0
    } else if wrapped < -180.0 {
        wrapped + 360.0
    } else {
        wrapped
    }
}

/// Format the clickable context line under a syntax error, if the error has one.
pub fn format_syntax_failure_context(e: &CommandError, command: &str) -> Option<Text> {
    let window = e.context_window()?;
    let suggest = Style {
        click: Some(text::ClickEvent::SuggestCommand(format!("/{command}"))),
        ..Style::default()
    };
    let mut context = Text::empty();
    if window.ellipsis {
        context.push("...");
    }
    context.push(window.before.as_str());
    if !window.after.is_empty() {
        context.push(
            Text::literal(window.after)
                .with_color(text::RED)
                .underlined(),
        );
    }
    context.push(
        Text::translate("command.context.here", &[])
            .with_color(text::RED)
            .italic(),
    );
    Some(context.with_color(text::GRAY).with_style(&suggest))
}

pub fn num(n: impl std::fmt::Display) -> Text {
    Text::literal(n.to_string())
}

impl crate::game::CommonGame<'_> {
    /// `EntityType.getDimensions().eyeHeight()` from `entity-types.json`, cached once.
    pub fn type_eye_height(&self, ty: &str) -> f64 {
        static EYE_HEIGHTS: std::sync::OnceLock<std::collections::HashMap<String, f64>> =
            std::sync::OnceLock::new();
        EYE_HEIGHTS
            .get_or_init(|| {
                std::fs::read(self.options.resources.join("entity-types.json"))
                    .ok()
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                    .and_then(|v| {
                        v.as_object().map(|o| {
                            o.iter()
                                .filter_map(|(k, t)| {
                                    Some((k.clone(), f64::from(t["eye_height"].as_f64()? as f32)))
                                })
                                .collect()
                        })
                    })
                    .unwrap_or_default()
            })
            .get(ty)
            .copied()
            .unwrap_or(0.0)
    }

    /// `VersionCommand.dumpVersion`'s lines: Sunrise reports the version it implements.
    pub fn version_lines(&self) -> Vec<Text> {
        let protocol = sunrise_protocol::PROTOCOL_VERSION;
        let mut lines: Vec<Text> = [
            ("commands.version.id", vec![Text::literal(VERSION_ID)]),
            (
                "commands.version.name",
                vec![Text::literal(VANILLA_VERSION)],
            ),
            ("commands.version.data", vec![num(DATA_VERSION)]),
            ("commands.version.series", vec![Text::literal("main")]),
            (
                "commands.version.protocol",
                vec![num(protocol), Text::literal(format!("0x{protocol:x}"))],
            ),
            (
                "commands.version.build_time",
                vec![Text::literal(BUILD_TIME)],
            ),
            (
                "commands.version.pack.resource",
                vec![Text::literal("97.1")],
            ),
            ("commands.version.pack.data", vec![Text::literal("121.0")]),
        ]
        .into_iter()
        .map(|(key, args)| Text::translate(key, &args))
        .collect();
        lines.push(Text::translate(
            if STABLE {
                "commands.version.stable.yes"
            } else {
                "commands.version.stable.no"
            },
            &[],
        ));
        lines
    }

    /// `Player.getDisplayName`: formatted team name with suggest, hover, and insertion.
    pub fn player_text(&self, name: &str, uuid: u128) -> Text {
        let shown = match self.score.board.players_team(name) {
            Some(team) => team.formatted_name(Text::literal(name)),
            None => Text::literal(name),
        };
        let mut shown = shown.with_component_fields(vec![
            (
                "click_event".into(),
                serde_json::json!({"action": "suggest_command", "command": format!("/tell {name} ")}),
            ),
            (
                "hover_event".into(),
                serde_json::json!({"action": "show_entity", "id": "minecraft:player",
                    "uuid": sunrise_protocol::crypto::uuid_string(uuid), "name": name}),
            ),
            ("insertion".into(), name.into()),
        ]);
        sunrise_sim::lan::hub::native_player_name_format(
            &self.loop_context,
            &Text::literal(name),
            &mut shown,
        );
        shown
    }

    /// The player's movement collides with the border as it is now (`CollisionGetter.borderCollision`).
    pub fn player_border(&mut self) {
        if let Some(p) = self.player.as_mut() {
            p.border = Some(self.border.geometry().clone());
        }
    }

    pub fn data_block_name(&self, p: [i32; 3]) -> String {
        let state = self.world.block(p[0], p[1], p[2]);
        self.pack
            .assets
            .registry
            .state(state)
            .map_or_else(|| "minecraft:air".into(), |s| s.name.to_string())
    }
}

pub trait CommandsCommonGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// `ItemStack.getDisplayName`: hover name in brackets, coloured and carrying show_item hover.
    fn stack_display_name(&self, s: &ItemStack) -> Text {
        let item = &*s.item;
        let empty = item == "minecraft:air" || s.count == 0;
        let custom = s.data().and_then(|data| data.custom_name.as_ref());
        let hover = self.stack_hover_name_text(s);
        // Component.empty().append(getHoverName()), italic for a custom name.
        let mut name = Text::empty().append(hover);
        if custom.is_some() {
            name = name.italic();
        }
        let wrapped = Text::translate("chat.square_brackets", &[name]);
        if empty {
            return wrapped;
        }

        // HoverEvent.ShowItem: the count only when it is not one.
        let mut shown = serde_json::json!({"action": "show_item", "id": item});
        if s.count != 1 {
            shown["count"] = s.count.into();
        }
        // ItemStackTemplate.fromNonEmptyStack keeps the stack's component patch.
        if let Some(patch) = sunrise_items::item_data::encode_components(s, self.items())
            && let Ok(components) = sunrise_core::json_nbt::tag_to_json(&patch)
        {
            shown["components"] = components;
        }
        wrapped
            .with_color(self.stack_rarity_color(s))
            .with_component_fields(vec![("hover_event".into(), shown)])
    }

    /// `ItemStack.getMaxStackSize`: the `max_stack_size` component, else the item's.
    fn stack_max_size(&self, s: &ItemStack) -> u32 {
        if let Some(Tag::Int(n)) = s.data().and_then(|d| d.other("minecraft:max_stack_size")) {
            return (*n).clamp(1, 99) as u32;
        }
        self.items().get(&s.item).map_or(64, |i| i.max_stack)
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> CommandsCommonGameExt1<'__sg> for G {}

/// Whether `uuid` is the player in place (`Target::Player`): during a guest's turn that is the guest, not the
/// level's own player (the world owner), whom `Level.getEntity(UUID)` finds only while they are in the level.
pub fn is_player_in_place(
    uuid: [i32; 4],
    acting: Option<u128>,
    level_player: Option<[i32; 4]>,
) -> bool {
    match acting {
        Some(guest) => sunrise_sim::lan::playerdata::ints_uuid(uuid) == guest,
        None => Some(uuid) == level_player,
    }
}

/// What the model keeps outside a stored block entity's `encoded` form
/// (`BlockEntity.saveWithFullMetadata`): the lock, the hopper's
/// `TransferCooldown` (`HopperBlockEntity.saveAdditional`), the custom name
/// and the pending loot table.
pub fn stored_block_entity_data(
    world: &sunrise_level::world::World,
    key: (i32, i32, i32),
    name: &str,
    entity: &BlockEntity,
    items: &sunrise_items::items::Items,
    hopper_cooldown: impl FnOnce() -> i32,
) -> Tag {
    let pending = world
        .chunks
        .get(&(key.0 >> 4, key.2 >> 4))
        .and_then(|chunk| chunk.saved.as_ref())
        .and_then(|saved| saved.pending_loot.get(&key));
    save::block_entity_encode::data_with(
        key,
        entity,
        name,
        save::block_entity_encode::DataFields {
            lock: world.block_lock(key),
            custom_name: world.block_names.get(&key),
            pending,
            hopper_cooldown: (name == "minecraft:hopper").then(hopper_cooldown),
        },
        items,
        |_| None,
    )
}

/// One addressable slot.
#[derive(Clone, Debug, PartialEq)]
pub enum Slot {
    /// `inventory[0..36]`: hotbar then the main rows.
    Inv(usize),
    /// The selected hotbar slot as `weapon.mainhand` names it: the equipment's
    /// `setItemSlot`, where `hotbar.N` is the inventory's `setItem`.
    MainHand,
    /// `armor[0..4]` (head, chest, legs, feet).
    Armor(usize),
    Offhand,
    /// The player's body or saddle (`EquipmentSlot` ordinal, `Game::equipment`).
    Equipment(usize),
    Ender(usize),
    /// A mob's equipment by name (`mainhand`, `offhand`, `head`, ...).
    Mob(u32, &'static str),
    /// An armor stand's equipment (`EquipmentSlot` ordinal).
    Stand(u32, usize),
    /// The ender dragon's equipment (`EquipmentSlot` ordinal).
    Dragon(u32, usize),
    Frame(u32),
    /// A dropped item's stack: `ItemEntity.getSlot(0)`, the `contents` slot.
    Item(u32),
    /// A chest or hopper minecart's slot (`ContainerEntity.getChestVehicleSlot`).
    Cart(u32, usize),
    Block([i32; 3], usize),
    /// A slot of another player (`for_player`): reached with their state in place.
    Guest(u32, Box<Slot>),
    /// A component collection for writes, or an indexed component item for reads.
    Contents(Box<Slot>, Component),
    ContentItem(Box<Slot>, Component, usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Component {
    Container,
    Bundle,
    Charged,
}

impl Component {
    pub fn named(name: &str) -> Option<Self> {
        match name.strip_prefix("minecraft:").unwrap_or(name) {
            "container" => Some(Self::Container),
            "bundle_contents" => Some(Self::Bundle),
            "charged_projectiles" => Some(Self::Charged),
            _ => None,
        }
    }
    /// The component's slots as a command sees them: a bundle's are its effective contents
    /// (its own list, or its item's default), the same list the menus and the tooltip read.
    pub fn items(
        self,
        catalog: &sunrise_items::items::Items,
        stack: &ItemStack,
    ) -> Vec<Option<ItemStack>> {
        if self == Self::Bundle {
            return sunrise_items::bundle::contents_of(catalog, stack)
                .map_or_else(Vec::new, |inside| {
                    inside.iter().cloned().map(Some).collect()
                });
        }
        let Some(data) = stack.data() else {
            return Vec::new();
        };
        match self {
            Self::Container => {
                let size = data
                    .container
                    .iter()
                    .map(|(i, _)| usize::from(*i) + 1)
                    .max()
                    .unwrap_or(0);
                let mut out = vec![None; size];
                for (i, item) in &data.container {
                    out[usize::from(*i)] = Some(item.clone());
                }
                out
            }
            Self::Bundle => unreachable!("a bundle's slots are read above"),
            Self::Charged => data.charged.iter().cloned().map(Some).collect(),
        }
    }
    /// ComponentSlotCollection writes a single normalized component back to
    /// its live parent; all unrelated parent components remain modeled.
    pub fn store(
        self,
        catalog: &sunrise_items::items::Items,
        stack: &mut ItemStack,
        mut slots: Vec<Option<ItemStack>>,
    ) {
        for slot in &mut slots {
            if slot
                .as_ref()
                .is_some_and(|s| s.count == 0 || &*s.item == "minecraft:air")
            {
                *slot = None;
            }
        }
        if self == Self::Bundle {
            // BundleContents.Mutable.mergeIdenticalStacks: merge into a later
            // matching entry, preserving the order of the surviving entries.
            let mut i = 0;
            while i < slots.len() {
                let Some(item) = slots[i].as_ref() else {
                    slots.remove(i);
                    continue;
                };
                if let Some(j) = (i + 1..slots.len()).find(|&j| {
                    slots[j]
                        .as_ref()
                        .is_some_and(|s| s.same_item_same_components(item))
                }) {
                    let count = item.count;
                    slots[j].as_mut().unwrap().count += count;
                    slots.remove(i);
                } else {
                    i += 1;
                }
            }
        }
        if self == Self::Bundle {
            // The same prototype-aware write as the menus: clearing the last slot leaves an
            // effective empty component.
            sunrise_items::bundle::set_contents(
                catalog,
                stack,
                slots.into_iter().flatten().collect(),
            );
            return;
        }
        let data = stack.data_mut();
        match self {
            Self::Container => {
                data.container = slots
                    .into_iter()
                    .enumerate()
                    .filter_map(|(i, s)| s.map(|s| (i as u8, s)))
                    .collect()
            }
            Self::Bundle => unreachable!("a bundle is written above"),
            Self::Charged => data.charged = slots.into_iter().flatten().collect(),
        }
        stack.tidy();
    }
}

impl crate::game::CommonGame<'_> {
    pub fn decorate_entity_display_name_with_uuid(
        kind: &str,
        uuid: Option<[i32; 4]>,
        team: Option<&sunrise_game_data::scoreboard::Team>,
        name: Text,
    ) -> Text {
        let shown = match team {
            Some(team) => team.formatted_name(name.clone()),
            None => name.clone(),
        };
        let Some(uuid) = uuid else {
            return shown;
        };
        let uuid = sunrise_sim::save::uuid_text(uuid);
        let kind = sunrise_command::args::normalize(kind);
        shown.set_component_fields(vec![
            (
                "hover_event".into(),
                serde_json::json!({
                    "action": "show_entity",
                    "id": kind,
                    "uuid": uuid,
                    "name": name.component(),
                }),
            ),
            ("insertion".into(), uuid.into()),
        ])
    }

    /// Decoded `CustomName` with `Entity.removeAction` applied, or `None` when
    /// absent or invalid under vanilla's `ComponentSerialization.CODEC`.
    fn custom_name_of(&self, id: u32) -> Option<Text> {
        let raw = self.persist_raw(id)?.get("CustomName")?;
        let decoded = sunrise_sim::entity_nbt::decode_custom_name_text(raw)?;
        Some(sunrise_game_data::scoreboard::remove_click_events(&decoded))
    }

    pub(in crate::game) fn bolt_plain_name(&self, id: u32) -> Text {
        if let Some(name) = self
            .bolts
            .iter()
            .find(|b| b.id == id)
            .and_then(|b| b.custom_name.as_ref())
            .and_then(sunrise_sim::entity_nbt::decode_custom_name_text)
        {
            return sunrise_game_data::scoreboard::remove_click_events(&name);
        }
        Text::translate("entity.minecraft.lightning_bolt", &[])
    }

    /// Shared `Entity.getDisplayName` decoration: `PlayerTeam.formatNameForTeam`
    /// of `getName()`, then `withHoverEvent(createHoverEvent())` (a `ShowEntity`
    /// of the type, UUID and name) and `withInsertion(getStringUUID())`.
    fn decorate_entity_display_name(
        &self,
        id: u32,
        kind: &str,
        team: Option<&sunrise_game_data::scoreboard::Team>,
        name: Text,
    ) -> Text {
        Self::decorate_entity_display_name_with_uuid(kind, self.persist.uuid_of(id), team, name)
    }
}

pub trait CommandsCommonGameExt2<'__sg>: crate::game::SideGame<'__sg> {
    /// The guest `conn` of the server’s LAN hub.
    fn command_guest<'__loan>(&'__loan self, conn: u32) -> Option<&'__loan Guest>
    where
        '__sg: '__loan,
    {
        crate::game::side_ops::player::SideOps::side_command_guest(self.side_state(), conn)
    }

    /// `Entity.getYRot` and `getXRot`: the entity's angles, not the direction its model faces.
    fn target_rotation(&self, t: Target) -> [f32; 2] {
        use crate::game::entity::Entity;
        use sunrise_sim::entity_nbt::loaded_rotation;
        let saved_rotation = |id| self.persist.raw.get(&id).map_or([0.0; 2], loaded_rotation);
        match t {
            Target::Player => [self.yaw, self.pitch],
            Target::Guest(conn) => self
                .command_guest(conn)
                .map_or([0.0; 2], |g| [g.yaw, g.pitch]),
            Target::Mob(id) => self
                .mobs
                .iter()
                .find(|m| m.id == id)
                .map_or([0.0; 2], Entity::rotation),
            Target::Item(id) => self
                .dropped
                .iter()
                .find(|a| a.id == id)
                .map_or([0.0; 2], Entity::rotation),
            Target::Orb(id) => self
                .orbs
                .iter()
                .find(|a| a.id == id)
                .map_or([0.0; 2], Entity::rotation),
            Target::Projectile(id) => self
                .creatures
                .projectiles
                .iter()
                .find(|a| a.id == id)
                .map_or([0.0; 2], Entity::rotation),
            Target::Stand(id) => self
                .armor_stands
                .iter()
                .find(|a| a.id == id)
                .map_or([0.0; 2], Entity::rotation),
            Target::Frame(id) => self
                .item_frames
                .iter()
                .find(|a| a.id == id)
                .map_or([0.0; 2], sunrise_sim::entities::ItemFrame::entity_rotation),
            Target::Painting(id) => self
                .paintings
                .iter()
                .find(|a| a.id == id)
                .map_or([0.0; 2], |a| {
                    [f32::from(a.facing) * 90.0, saved_rotation(id)[1]]
                }),
            Target::Boat(id) => self
                .boats
                .iter()
                .find(|b| b.id == id)
                .map_or([0.0; 2], Entity::rotation),
            Target::Cart(id) => self
                .carts
                .iter()
                .find(|c| c.id == id)
                .map_or([0.0; 2], Entity::rotation),
            Target::Fangs(id) => self
                .fangs
                .list
                .iter()
                .find(|f| f.id == id)
                .map_or([0.0; 2], |f| [f.yaw, saved_rotation(id)[1]]),
            Target::Crystal(id) => self
                .crystals
                .iter()
                .find(|c| c.id == id)
                .map_or([0.0; 2], |c| c.rotation),
            Target::Cloud(id) => saved_rotation(id),
            Target::Bolt(id) => self
                .bolts
                .iter()
                .find(|b| b.id == id)
                .map_or([0.0; 2], |b| b.rotation),
            Target::Dragon(id) => self
                .dragon
                .as_ref()
                .filter(|a| a.id == id)
                .map_or([0.0; 2], |a| [a.yaw, a.pitch]),
            Target::Tnt(id) => self
                .tnts
                .iter()
                .find(|a| a.id == id)
                .map_or([0.0; 2], Entity::rotation),
            Target::Cushion(id) => self
                .cushions
                .iter()
                .find(|a| a.id == id)
                .map_or([0.0; 2], |a| [a.yaw, saved_rotation(id)[1]]),
            Target::Falling(id) => self
                .falling
                .iter()
                .find(|a| a.id == id)
                .map_or([0.0; 2], Entity::rotation),
            Target::Rocket(id) => self
                .rockets
                .iter()
                .find(|a| a.id == id)
                .map_or([0.0; 2], Entity::rotation),
            Target::Trident(id) => self
                .tridents
                .iter()
                .find(|a| a.id == id)
                .map_or([0.0; 2], Entity::rotation),
            Target::Spawner(id) => self
                .ominous_spawners
                .iter()
                .find(|a| a.id == id)
                .and_then(|a| a.raw.as_deref())
                .map_or([0.0; 2], sunrise_sim::entity_nbt::rotation),
            Target::Hook(id) => {
                <Self::State as crate::game::side_ops::items::SideOps>::side_command_hook_rotation(
                    self, id,
                )
                .unwrap_or([0.0; 2])
            }
            Target::Other(id) => {
                crate::game::side_ops::player::SideOps::side_other_entity_rotation(
                    self.side_state(),
                    id,
                )
                .unwrap_or([0.0; 2])
            }
            Target::General(id) => self
                .resident_entities
                .general
                .row(id)
                .map(|row| row.rotation)
                .or_else(|| {
                    crate::game::side_ops::mobs_rest::SideOps::side_remote_general_rotation(
                        self.side_state(),
                        id,
                    )
                })
                .unwrap_or([0.0; 2]),
            // Dragon parts aren't admitted by UUID. Natural fence knots never turn.
            Target::DragonPart(_) | Target::Knot(_) => [0.0; 2],
        }
    }

    /// `ServerLevel.getEntity(UUID)`: any entity of the level in place, not
    /// only mobs (displays, armor stands, items...). The map is keyed by id,
    /// so a killed entity and a later summon of the same UUID can both be
    /// in it; the one the level still holds is the entity.
    fn target_with_uuid(&self, uuid: [i32; 4]) -> Option<Target> {
        if is_player_in_place(
            uuid,
            crate::game::Side::lan_acting_uuid(self.side_state()),
            self.level.player_uuid,
        ) {
            if crate::game::Side::native_command_filter(self.side_state()) {
                return <Self::State as crate::game::side_ops::player::SideOps>::side_filtered_command_player_target(self, uuid);
            }
            return Some(Target::Player);
        }
        self.persist.uuids.iter().find_map(|(&id, found)| {
            if *found != uuid {
                return None;
            }
            self.level_entity(i32::try_from(id).ok()?)
        })
    }

    /// `Level.getEntity`: the entity of the level in place with a network
    /// id, players aside.
    fn level_entity(&self, id: i32) -> Option<Target> {
        let id = id as u32;
        match <Self::State as crate::game::side_ops::world::SideOps>::side_server_resident_entity(
            self, id,
        ) {
            sunrise_sim::join_seq::residency::table::EntityLookup::Found { location, value }
                if location.owner.dimension == self.dim.key()
                    && !matches!(
                        location.owner.kind,
                        sunrise_sim::join_seq::residency::Kind::Player
                            | sunrise_sim::join_seq::residency::Kind::Hook
                            | sunrise_sim::join_seq::residency::Kind::DragonPart
                    ) =>
            {
                Some(value)
            }
            // A client's own copy of a row its server shows it.
            _ => crate::game::side_ops::mobs_rest::SideOps::side_remote_general_type_slot(
                self.side_state(),
                id,
            )
            .map(|_| Target::General(id)),
        }
    }

    fn entity_tag_matches(&self, tag: &str, ty: &str) -> bool {
        let tag = sunrise_command::args::normalized(tag);
        self.items().entity_type_tags.contains(&tag, ty)
    }

    /// The name of the player the game acts for: a guest's own while their
    /// command or action runs (`Player.getName`).
    fn acting_player_name<'__loan>(&'__loan self) -> &'__loan str
    where
        '__sg: '__loan,
    {
        crate::game::Side::lan_acting_name(self.side_state()).unwrap_or(PLAYER_NAME)
    }

    /// Entity.getDisplayName.
    fn target_name(&self, t: Target) -> Text {
        match t {
            Target::Player => {
                let name = self.acting_player_name();
                if let Some(uuid) = crate::game::Side::lan_acting_uuid(self.side_state()) {
                    return self.player_text(name, uuid);
                }
                match self.score.board.players_team(name) {
                    // PlayerTeam.formatNameForTeam.
                    Some(team) => team.formatted_name(Text::literal(name)),
                    None => Text::literal(name),
                }
            }
            Target::Guest(conn) => match self.command_guest(conn) {
                Some(g) => self.player_text(&g.name, g.uuid),
                None => Text::empty(),
            },
            Target::Mob(id) => self.mob_command_name(id),
            other => self.entity_display_name(other),
        }
    }

    /// `PlayerList.getPlayers` but the player in place: the guests who
    /// joined the world.
    fn command_players<'__loan>(&'__loan self) -> impl Iterator<Item = &'__loan Guest>
    where
        '__sg: '__loan,
    {
        let acting = crate::game::Side::lan_acting(self.side_state());
        crate::game::side_ops::lan::SideOps::side_command_players(self.side_state())
            .filter(move |g| Some(g.conn) != acting && self.native_command_player_available(g))
    }

    /// Native console calls cannot reach an outer turn's displaced player.
    /// Ordinary command resolution is unchanged, including nested turns.
    fn native_command_player_available(&self, guest: &Guest) -> bool {
        !crate::game::Side::native_command_filter(self.side_state())
            || self.entity(guest.entity_id as u32).is_some()
    }

    /// `EntitySelector.findEntities` with a UUID: the player with it, else
    /// the entity.
    fn target_by_uuid(&self, uuid: u128) -> Option<Target> {
        let own = crate::game::Side::lan_acting_uuid(self.side_state()).or_else(|| {
            self.level.player_uuid.map(|[a, b, c, d]| {
                (u128::from(a as u32) << 96)
                    | (u128::from(b as u32) << 64)
                    | (u128::from(c as u32) << 32)
                    | u128::from(d as u32)
            })
        });
        if self.player.is_some() && own == Some(uuid) {
            return (!crate::game::Side::lan_acting_fake(self.side_state()))
                .then_some(Target::Player);
        }
        if let Some(g) = self.command_players().find(|g| g.uuid == uuid) {
            return Some(Target::Guest(g.conn));
        }
        let words = [96, 64, 32, 0].map(|shift| (uuid >> shift) as u32 as i32);
        self.target_with_uuid(words)
    }

    fn target_type(&self, t: Target) -> String {
        match t {
            Target::Player | Target::Guest(_) => "minecraft:player".into(),
            Target::Mob(id) => self
                .mobs
                .iter()
                .find(|m| m.id == id)
                .map_or_else(String::new, |m| {
                    sunrise_command::args::normalize(&m.kind.id)
                }),
            Target::Item(_) => "minecraft:item".into(),
            Target::Orb(_) => "minecraft:experience_orb".into(),
            Target::Projectile(id) => self
                .creatures
                .projectiles
                .iter()
                .find(|p| p.id == id)
                .map_or_else(String::new, |p| p.entity_type().into()),
            Target::Stand(_) => "minecraft:armor_stand".into(),
            Target::General(id) => self
                .general_kind(id)
                .map_or_else(String::new, str::to_owned),
            Target::Frame(id) => {
                if self.item_frames.iter().any(|f| f.id == id && f.glow) {
                    "minecraft:glow_item_frame".into()
                } else {
                    "minecraft:item_frame".into()
                }
            }
            Target::Painting(_) => "minecraft:painting".into(),
            Target::Boat(id) => self
                .boats
                .iter()
                .find(|b| b.id == id)
                .map_or_else(|| "minecraft:oak_boat".into(), |b| b.entity_type()),
            Target::Cart(id) => self
                .carts
                .iter()
                .find(|c| c.id == id)
                .map_or_else(|| "minecraft:minecart".into(), |c| c.kind.id().into()),
            Target::Other(id) => crate::game::side_ops::player::SideOps::side_other_entity_kind(
                self.side_state(),
                id,
            )
            .map_or_else(String::new, str::to_owned),
            Target::Crystal(_) => "minecraft:end_crystal".into(),
            // `EnderDragonPart` takes its parent's type.
            Target::Dragon(_) | Target::DragonPart(_) => "minecraft:ender_dragon".into(),
            Target::Tnt(_) => "minecraft:tnt".into(),
            Target::Cushion(_) => "minecraft:cushion".into(),
            Target::Falling(_) => "minecraft:falling_block".into(),
            Target::Rocket(_) => "minecraft:firework_rocket".into(),
            Target::Trident(_) => "minecraft:trident".into(),
            Target::Fangs(_) => "minecraft:evoker_fangs".into(),
            Target::Spawner(_) => "minecraft:ominous_item_spawner".into(),
            Target::Cloud(_) => "minecraft:area_effect_cloud".into(),
            Target::Bolt(_) => "minecraft:lightning_bolt".into(),
            Target::Hook(_) => "minecraft:fishing_bobber".into(),
            Target::Knot(_) => "minecraft:leash_knot".into(),
        }
    }

    /// `Entity.getName`: the decoded `CustomName` with click actions removed,
    /// else the entity's type name, free of team formatting, hover or insertion.
    fn target_base_name(&self, t: Target) -> Text {
        match t {
            Target::Player => Text::literal(self.acting_player_name()),
            Target::Guest(conn) => self
                .command_guest(conn)
                .map_or_else(Text::empty, |g| Text::literal(&g.name)),
            Target::Mob(id) => self
                .mobs
                .iter()
                .find(|m| m.id == id)
                .map_or_else(|| Text::translate("entity.", &[]), |m| m.command_name()),
            // ItemEntity.getName: its CustomName, else the stack's item name
            // (`getItemName`: a mod item's Part, else the item's own `getName`).
            Target::Item(id) => {
                if let Some(name) = self.custom_name_of(id) {
                    return name;
                }
                match self.dropped.iter().find(|i| i.id == id) {
                    Some(item) => self.stack_item_name_text(&item.stack),
                    None => Text::literal(self.pack.names.display_item("")),
                }
            }
            // FallingBlockEntity.getTypeName: `entity.minecraft.falling_block_type`
            // with the block's name.
            Target::Falling(id) => {
                let custom = self
                    .falling
                    .iter()
                    .find(|f| f.id == id)
                    .and_then(|f| f.saved.as_ref())
                    .and_then(|saved| saved.1.get("CustomName"))
                    .or_else(|| self.persist_raw(id).and_then(|r| r.get("CustomName")))
                    .and_then(sunrise_sim::entity_nbt::decode_custom_name_text)
                    .map(|text| sunrise_game_data::scoreboard::remove_click_events(&text));
                if let Some(name) = custom {
                    return name;
                }
                let block = self
                    .falling
                    .iter()
                    .find(|f| f.id == id)
                    .map(|f| self.res.facts().get(f.state).name.replace(':', "."))
                    .unwrap_or_default();
                let block = Text::translate(&format!("block.{block}"), &[]);
                Text::translate("entity.minecraft.falling_block_type", &[block])
            }
            Target::Bolt(id) => self.bolt_plain_name(id),
            // A general row keeps its name on the row, not in a saved compound.
            Target::General(id) => {
                match self
                    .resident_entities
                    .general
                    .row(id)
                    .and_then(|row| row.custom_name.as_ref())
                {
                    Some(name) => sunrise_game_data::scoreboard::remove_click_events(name),
                    None => Text::translate(
                        &format!("entity.{}", self.target_type(t).replace(':', ".")),
                        &[],
                    ),
                }
            }
            other => {
                if let Some(name) = crate::game::commands_common::target_id(other)
                    .and_then(|id| self.custom_name_of(id))
                {
                    return name;
                }
                let ty = self.target_type(other);
                Text::translate(&format!("entity.{}", ty.replace(':', ".")), &[])
            }
        }
    }

    /// `Entity.getDisplayName` for the mob `id`.
    fn mob_command_name(&self, id: u32) -> Text {
        let Some(m) = self.mobs.iter().find(|m| m.id == id) else {
            return Text::translate("entity.", &[]);
        };
        self.decorate_entity_display_name(id, &m.kind.id, self.mob_team_of(m), m.command_name())
    }

    /// `Entity.getDisplayName` for any non-player, non-mob target.
    fn entity_display_name(&self, t: Target) -> Text {
        let name = self.target_base_name(t);
        let Some(id) = crate::game::commands_common::target_id(t) else {
            return name;
        };
        let kind = self.target_type(t);
        self.decorate_entity_display_name(id, &kind, self.entity_team(id), name)
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> CommandsCommonGameExt2<'__sg> for G {}

use crate::game::tables::GameItems as _;

use crate::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use crate::game::remote::general::GeneralGameExt1 as _;

use crate::game::scoreboard_common::CommonGameExt1 as _;

use crate::game::stack_names::StackNamesGameExt1 as _;
