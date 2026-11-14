//! Argument types (Brigadier's and Minecraft's `ArgumentType`s): parsing with
//! the vanilla error messages and completion suggestions.
use super::{CmdResult, CommandError, Context, Reader, SuggestionsBuilder};
use horizon_client_core::command::{
    self, Coord, Coordinates, DoubleRange, IntRange, Selector, is_id_char, parse_coord_double,
    parse_coord_int, parse_local, parse_world, read_double_bounds, read_identifier,
    read_int_bounds, selector, suggest_resources,
};
use horizon_client_core::text::Text;
use std::{fmt, sync::Arc};

pub use horizon_client_core::registries::{
    CustomCompletionAction, Registries, RegistryIds, modify_custom_completions,
};
pub use horizon_core::game_mode::GameMode;

/// How much of the command line a registered custom argument takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CustomArgumentKind {
    /// One word, as Brigadier's `word` string: ASCII letters and digits, and `_ - . +`. It ends
    /// at the first other character, a space included.
    Word,
    /// The rest of the line, spaces included.
    Greedy,
}

/// One completion a registered custom argument offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomSuggestion {
    /// The text that replaces the typed prefix.
    pub text: String,
    /// An optional explanation shown beside the completion.
    pub tooltip: Option<String>,
}

/// A mod's registered argument type, as the dispatcher parses and completes it.
///
/// The mod API object lives above this crate; the game adapts it into these closures so command
/// parsing stays independent of the mod API.
#[derive(Clone)]
pub struct CustomArgument {
    id: Arc<str>,
    kind: CustomArgumentKind,
    parse: Arc<dyn Fn(&str) -> Result<String, Text> + Send + Sync>,
    suggest: Arc<dyn Fn(&str) -> Vec<CustomSuggestion> + Send + Sync>,
}

impl fmt::Debug for CustomArgument {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CustomArgument")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .finish()
    }
}

impl CustomArgument {
    /// A custom argument type `id` (such as `mymod:machine`) that reads a [`CustomArgumentKind`],
    /// validates it with `parse` and completes it with `suggest`.
    pub fn new(
        id: impl Into<Arc<str>>,
        kind: CustomArgumentKind,
        parse: impl Fn(&str) -> Result<String, Text> + Send + Sync + 'static,
        suggest: impl Fn(&str) -> Vec<CustomSuggestion> + Send + Sync + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            kind,
            parse: Arc::new(parse),
            suggest: Arc::new(suggest),
        }
    }
    /// The registered type's namespaced id.
    pub fn id(&self) -> &str {
        &self.id
    }
    /// How much of the line the argument takes.
    pub fn kind(&self) -> CustomArgumentKind {
        self.kind
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resource {
    EntityType,
    Effect,
    Enchantment,
    Structure,
    Biome,
    Clock,
    Timeline,
}
impl Resource {
    pub fn registry(self) -> &'static str {
        match self {
            Resource::EntityType => "minecraft:entity_type",
            Resource::Effect => "minecraft:mob_effect",
            Resource::Enchantment => "minecraft:enchantment",
            Resource::Structure => "minecraft:worldgen/structure",
            Resource::Biome => "minecraft:worldgen/biome",
            Resource::Clock => "minecraft:world_clock",
            Resource::Timeline => "minecraft:timeline",
        }
    }
}

#[derive(Clone, Debug)]
pub enum ArgType {
    Bool,
    Integer {
        min: i32,
        max: i32,
    },
    Float {
        min: f32,
        max: f32,
    },
    Double {
        min: f64,
        max: f64,
    },
    Word,
    Str,
    Greedy,
    Entity {
        single: bool,
        players: bool,
    },
    BlockPos,
    Vec3 {
        center: bool,
    },
    Rotation,
    Angle,
    Item,
    ItemPredicate,
    BlockState,
    BlockPredicate,
    Resource(Resource),
    /// A plain identifier; suggestions come from the named list.
    Id(&'static str),
    /// ResourceArgument over a registry (the element must exist).
    Res(&'static str),
    /// ResourceKeyArgument (checked when the command runs).
    ResKey(&'static str),
    /// ResourceOrTagArgument (elements and tags must exist).
    ResOrTag(&'static str),
    /// ResourceOrTagKeyArgument.
    ResOrTagKey(&'static str),
    /// StringRepresentableArgument-like: one of the names, with the error key.
    StrEnum(&'static [&'static str], &'static str),
    /// HexColorArgument.
    HexColor,
    /// UuidArgument.
    Uuid,
    /// GameProfileArgument.
    GameProfile,
    Time {
        min: i32,
    },
    GameMode,
    Message,
    Dimension,
    Anchor,
    /// SwizzleArgument: a set of `x`, `y` and `z` (kept as text).
    Swizzle,
    /// ComponentArgument: an SNBT/JSON text component (kept as JSON text).
    Component,
    /// StyleArgument: an SNBT style map.
    Style,
    /// CompoundTagArgument (kept as SNBT text).
    Nbt,
    /// ScoreHolderArgument.
    ScoreHolder {
        multiple: bool,
    },
    /// ObjectiveArgument (checked when the command runs).
    Objective,
    /// TeamArgument (checked when the command runs).
    Team,
    /// ObjectiveCriteriaArgument.
    Criteria,
    /// ScoreboardSlotArgument.
    ScoreboardSlot,
    /// OperationArgument.
    Operation,
    /// TeamColorArgument.
    TeamColor,
    /// RangeArgument.Ints (`1`, `1..`, `..5`, `1..5`).
    IntRange,
    FloatRange,
    /// NbtPathArgument (kept as text).
    NbtPath,
    /// NbtTagArgument: any SNBT value (kept as text).
    NbtTag,
    /// Vec2Argument: `x z` with `~` relative parts.
    Vec2,
    /// ColumnPosArgument: block `x z`.
    ColumnPos,
    /// ParticleArgument: a particle id and its options (kept as text).
    Particle,
    /// One of a fixed set of words.
    Enum(&'static [&'static str]),
    /// A (possibly quoted) name suggested from a registry list.
    Name(&'static str),
    /// FunctionArgument: `ns:path`, or `#ns:path` for a function tag.
    Function,
    /// SlotsArgument: a slot name or a wildcard group (`container.*`).
    Slots,
    /// SlotArgument: one slot (`container.5`, `weapon.mainhand`).
    Slot,
    /// SlotSourceArgument: a slot range name, or a slot source id or inline value.
    SlotSource,
    /// ResourceOrIdArgument over a registry: an id or an inline SNBT value.
    ResourceOrId(&'static str),
    /// ResourceSelectorArgument: a `*`/`?` pattern over a registry's ids.
    ResourceSelector(&'static str),
    /// A registered argument type: the mod's parser and suggestions.
    Custom(CustomArgument),
}

/// FilenameUtils.wildcardMatch (case-sensitive) of `*` and `?` patterns.
pub fn wildcard_match(text: &str, pattern: &str) -> bool {
    // Keep the backtracking cursors as UTF-8 byte offsets.  The old port
    // materialised both strings as `Vec<char>` for indexing, which made every
    // resource id allocate during a selector scan.  `chars().next()` keeps
    // the same Unicode scalar semantics for `?` while avoiding those copies.
    let (mut ti, mut pi) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ti < text.len() {
        let tc = text[ti..]
            .chars()
            .next()
            .expect("cursor always stays on a char boundary");
        if pi < pattern.len() {
            let pc = pattern[pi..]
                .chars()
                .next()
                .expect("cursor always stays on a char boundary");
            if pc == '?' || pc == tc {
                ti += tc.len_utf8();
                pi += pc.len_utf8();
                continue;
            }
            if pc == '*' {
                star = Some((pi, ti));
                pi += 1;
                continue;
            }
        }
        if let Some((s, mark)) = &mut star {
            pi = *s + 1;
            let c = text[*mark..]
                .chars()
                .next()
                .expect("backtracking cursor stays before text end");
            *mark += c.len_utf8();
            ti = *mark;
            continue;
        }
        return false;
    }
    while pi < pattern.len() && pattern.as_bytes()[pi] == b'*' {
        pi += 1;
    }
    pi == pattern.len()
}
/// ResourceSelectorArgument: the registry ids a pattern selects.
pub fn select_resources<'a>(ids: &'a [String], pattern: &str) -> Vec<&'a String> {
    let pattern = if pattern.contains(':') {
        pattern.to_owned()
    } else {
        format!("minecraft:{pattern}")
    };
    ids.iter()
        .filter(|id| wildcard_match(id, &pattern))
        .collect()
}

/// SlotRanges.SLOTS: every slot range name and its size, in vanilla order.
pub fn slot_ranges() -> Vec<(String, usize)> {
    let mut v: Vec<(String, usize)> = Vec::new();
    let range = |v: &mut Vec<(String, usize)>, prefix: &str, n: usize| {
        for i in 0..n {
            v.push((format!("{prefix}{i}"), 1));
        }
        v.push((format!("{prefix}*"), n));
    };
    v.push(("contents".into(), 1));
    range(&mut v, "container.", 54);
    range(&mut v, "hotbar.", 9);
    range(&mut v, "inventory.", 27);
    range(&mut v, "enderchest.", 27);
    range(&mut v, "mob.inventory.", 8);
    range(&mut v, "horse.", 15);
    for (name, n) in [
        ("weapon", 1),
        ("weapon.mainhand", 1),
        ("weapon.offhand", 1),
        ("weapon.*", 2),
        ("armor.head", 1),
        ("armor.chest", 1),
        ("armor.legs", 1),
        ("armor.feet", 1),
        ("armor.body", 1),
        ("armor.*", 5),
        ("saddle", 1),
        ("horse.chest", 1),
        ("player.cursor", 1),
    ] {
        v.push((name.to_owned(), n));
    }
    range(&mut v, "player.crafting.", 4);
    v
}
/// Single-slot names (SlotRanges.singleSlotNames).
pub fn slot_names() -> Vec<String> {
    slot_ranges()
        .into_iter()
        .filter(|(_, n)| *n == 1)
        .map(|(s, _)| s)
        .collect()
}
pub fn integer(min: i32) -> ArgType {
    ArgType::Integer { min, max: i32::MAX }
}

/// A `ScoreHolderArgument` value.
#[derive(Clone, Debug, PartialEq)]
pub enum ScoreHolderArg {
    Wildcard,
    Name(String),
    Selector(Selector),
}
#[derive(Clone, Debug, PartialEq)]
pub struct BlockInput {
    pub id: String,
    pub properties: Vec<(String, String)>,
    /// `#tag` for block predicates.
    pub tag: bool,
    pub nbt: Option<String>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ItemInput {
    pub id: String,
    pub tag: bool,
    pub any: bool,
    pub components: Option<String>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    Bool(bool),
    Int(i32),
    Float(f32),
    Double(f64),
    Str(String),
    Entity(Selector),
    Coords(Coordinates),
    Angle(Coord),
    Item(ItemInput),
    Block(BlockInput),
    Resource(String),
    GameMode(GameMode),
    /// `eyes` (true) or `feet`.
    Anchor(bool),
    ScoreHolder(ScoreHolderArg),
    IntRange(IntRange),
    DoubleRange(DoubleRange),
    /// A custom argument: the id of its registered type and its canonical value.
    Custom {
        kind: Arc<str>,
        value: String,
    },
}

/// Identifier.read: the longest run of identifier characters, normalised to
/// the `minecraft` namespace (an empty path is valid).
pub fn read_id(reader: &mut Reader) -> CmdResult<String> {
    read_identifier(reader)
}
pub fn normalize(id: &str) -> String {
    normalized(id).into_owned()
}
/// `normalize` that borrows an id which already has its namespace.
pub fn normalized(id: &str) -> std::borrow::Cow<'_, str> {
    match id.split_once(':') {
        Some(("", path)) => format!("minecraft:{path}").into(),
        Some(_) => id.into(),
        None => format!("minecraft:{id}").into(),
    }
}
/// String.split(" "): trailing empty strings are dropped.
fn java_split(s: &str) -> Vec<&str> {
    let mut parts: Vec<&str> = s.split(' ').collect();
    while parts.len() > 1 && parts.last() == Some(&"") {
        parts.pop();
    }
    if parts == [""] && !s.is_empty() {
        parts.clear();
    }
    parts
}
/// SharedSuggestionProvider.suggestCoordinates / suggest2DCoordinates: the
/// candidates `ty` parses completely. `relevant` arguments (block and column
/// positions) offer the targeted block (`getRelevantCoordinates`), the others
/// (`Vec3`, `Vec2`) the exact hit point (`getAbsoluteCoordinates`).
fn suggest_coordinates(
    builder: &mut SuggestionsBuilder,
    reg: &Registries,
    ty: &ArgType,
    relevant: bool,
) {
    let remaining = builder.remaining().to_owned();
    let three = matches!(ty, ArgType::BlockPos | ArgType::Vec3 { .. });
    let coords: [String; 3] = if remaining.starts_with('^') {
        ["^".into(), "^".into(), "^".into()]
    } else if let (true, Some(t)) = (relevant, reg.target_block) {
        [t[0].to_string(), t[1].to_string(), t[2].to_string()]
    } else if let (false, Some(p)) = (relevant, &reg.target_point) {
        p.clone()
    } else {
        ["~".into(), "~".into(), "~".into()]
    };
    let valid = |text: &str| {
        let mut r = Reader::new(text);
        ty.parse(&mut r, reg).is_ok()
    };
    let mut result: Vec<String> = Vec::new();
    let [x, y, z] = &coords;
    if remaining.is_empty() {
        let full = if three {
            format!("{x} {y} {z}")
        } else {
            format!("{x} {z}")
        };
        if valid(&full) {
            result.push(x.clone());
            if three {
                result.push(format!("{x} {y}"));
            }
            result.push(full);
        }
    } else {
        let fields = java_split(&remaining);
        match (fields.len(), three) {
            (1, true) => {
                let full = format!("{} {y} {z}", fields[0]);
                if valid(&full) {
                    result.push(format!("{} {y}", fields[0]));
                    result.push(full);
                }
            }
            (2, true) => {
                let full = format!("{} {} {z}", fields[0], fields[1]);
                if valid(&full) {
                    result.push(full);
                }
            }
            (1, false) => {
                let full = format!("{} {z}", fields[0]);
                if valid(&full) {
                    result.push(full);
                }
            }
            _ => {}
        }
    }
    builder.suggest_matching(result.iter().map(String::as_str));
}

pub const OPERATIONS: [&str; 9] = ["=", "+=", "-=", "*=", "/=", "%=", "<", ">", "><"];
/// `NbtPathArgument.parse`, sharing the executable lower NBT-path grammar.
fn parse_nbt_path(reader: &mut Reader) -> CmdResult<()> {
    horizon_client_core::nbt_path::NbtPath::parse_reader(reader).map(|_| ())
}
/// The required fields of each particle type's options codec (declaration order).
fn particle_fields(id: &str) -> &'static [&'static str] {
    match id.trim_start_matches("minecraft:") {
        "block" | "block_marker" | "falling_dust" | "dust_pillar" | "block_crumble" => {
            &["block_state"]
        }
        "geyser" | "geyser_plume" => &["water_blocks"],
        "geyser_base" | "geyser_poof" => &["water_blocks", "burst_impulse_base"],
        "dust" => &["color", "scale"],
        "dust_color_transition" => &["from_color", "to_color", "scale"],
        "entity_effect" | "tinted_leaves" | "flash" => &["color"],
        "sculk_charge" => &["roll"],
        "item" => &["item"],
        "vibration" => &["destination", "arrival_in_ticks"],
        "trail" => &["target", "color", "duration"],
        "shriek" => &["delay"],
        _ => &[],
    }
}
/// The particle type's codec over the options (MapCodec field errors, newest first).
fn particle_options(id: &str, t: &horizon_core::nbt::Tag) -> Result<(), String> {
    use super::codec::{get, no_key, not_map, snbt};
    let horizon_core::nbt::Tag::Compound(map) = t else {
        return Err(not_map(t));
    };
    let missing: Vec<String> = particle_fields(id)
        .iter()
        .rev()
        .filter(|f| get(map, f).is_none())
        .map(|f| no_key(f, t))
        .collect();
    if !missing.is_empty() {
        return Err(missing.join("; "));
    }
    if particle_fields(id) == ["block_state"]
        && let Some(state) = get(map, "block_state")
        && !matches!(state, horizon_core::nbt::Tag::String(_))
    {
        // withAlternative(BlockState.CODEC, the block id codec).
        let first = match state {
            horizon_core::nbt::Tag::Compound(_) => {
                super::codec::either("Not a string".into(), no_key("id", state))
            }
            _ => not_map(state),
        };
        let _ = snbt;
        return Err(super::codec::either(first, "Not a string".into()));
    }
    Ok(())
}
/// TimeArgument units.
fn time_unit(unit: &str) -> Option<i32> {
    match unit {
        "" | "t" => Some(1),
        "s" => Some(20),
        "d" => Some(24000),
        _ => None,
    }
}

impl ArgType {
    pub fn parse(&self, reader: &mut Reader, reg: &Registries) -> CmdResult<Arg> {
        let start = reader.cursor;
        Ok(match self {
            ArgType::Bool => Arg::Bool(reader.read_boolean()?),
            ArgType::Integer { min, max } => {
                let v = reader.read_int()?;
                if v < *min {
                    reader.cursor = start;
                    return Err(CommandError::tr_at(
                        "argument.integer.low",
                        &[min, &v],
                        reader,
                    ));
                }
                if v > *max {
                    reader.cursor = start;
                    return Err(CommandError::tr_at(
                        "argument.integer.big",
                        &[max, &v],
                        reader,
                    ));
                }
                Arg::Int(v)
            }
            ArgType::Float { min, max } => {
                let v = reader.read_float()?;
                if v < *min {
                    reader.cursor = start;
                    return Err(CommandError::tr_at(
                        "argument.float.low",
                        &[
                            &horizon_client_core::snbt::java_float(*min),
                            &horizon_client_core::snbt::java_float(v),
                        ],
                        reader,
                    ));
                }
                if v > *max {
                    reader.cursor = start;
                    return Err(CommandError::tr_at(
                        "argument.float.big",
                        &[
                            &horizon_client_core::snbt::java_float(*max),
                            &horizon_client_core::snbt::java_float(v),
                        ],
                        reader,
                    ));
                }
                Arg::Float(v)
            }
            ArgType::Double { min, max } => {
                let v = reader.read_double()?;
                if v < *min {
                    reader.cursor = start;
                    return Err(CommandError::tr_at(
                        "argument.double.low",
                        &[
                            &horizon_client_core::snbt::java_double(*min),
                            &horizon_client_core::snbt::java_double(v),
                        ],
                        reader,
                    ));
                }
                if v > *max {
                    reader.cursor = start;
                    return Err(CommandError::tr_at(
                        "argument.double.big",
                        &[
                            &horizon_client_core::snbt::java_double(*max),
                            &horizon_client_core::snbt::java_double(v),
                        ],
                        reader,
                    ));
                }
                Arg::Double(v)
            }
            ArgType::Word => Arg::Str(reader.read_unquoted_string().to_owned()),
            ArgType::Str => Arg::Str(reader.read_string()?),
            ArgType::Greedy => {
                let s = reader.remaining().to_owned();
                reader.cursor = reader.string.len();
                Arg::Str(s)
            }
            ArgType::Message => {
                // MessageArgument.Message.parseText.
                let len = reader.remaining().encode_utf16().count();
                if len > 256 {
                    return Err(CommandError::tr("argument.message.too_long", &[&len, &256]));
                }
                let s = reader.remaining().to_owned();
                if reg.permission >= 2 {
                    while reader.can_read() {
                        if reader.peek() == '@' {
                            let at = reader.cursor;
                            if let Err(e) = selector::parse(reader, reg, true) {
                                let key = e.key.as_str();
                                if key != "argument.entity.selector.missing"
                                    && key != "argument.entity.selector.unknown"
                                {
                                    return Err(e);
                                }
                                reader.cursor = at + 1;
                            }
                        } else {
                            reader.skip();
                        }
                    }
                }
                reader.cursor = reader.string.len();
                Arg::Str(s)
            }
            ArgType::Entity { single, players } => {
                let s = selector::parse(reader, reg, reg.permission >= 2)?;
                // Vanilla rewinds to the very start of the input for these.
                if s.limit > 1 && *single {
                    reader.cursor = 0;
                    let key = if *players {
                        "argument.player.toomany"
                    } else {
                        "argument.entity.toomany"
                    };
                    return Err(CommandError::tr_at(key, &[], reader));
                }
                if s.include_entities && *players && !s.is_self() {
                    reader.cursor = 0;
                    return Err(CommandError::tr_at("argument.player.entities", &[], reader));
                }
                Arg::Entity(s)
            }
            ArgType::BlockPos => Arg::Coords(command::parse_block_pos(reader)?),
            ArgType::Vec3 { center } => {
                if reader.can_read() && reader.peek() == '^' {
                    Arg::Coords(parse_local(reader)?)
                } else {
                    Arg::Coords(parse_world(reader, false, *center)?)
                }
            }
            ArgType::Rotation => {
                if !reader.can_read() {
                    return Err(CommandError::tr_at(
                        "argument.rotation.incomplete",
                        &[],
                        reader,
                    ));
                }
                let yaw = parse_coord_double(reader, false)?;
                if !reader.can_read() || reader.peek() != ' ' {
                    reader.cursor = start;
                    return Err(CommandError::tr_at(
                        "argument.rotation.incomplete",
                        &[],
                        reader,
                    ));
                }
                reader.skip();
                let pitch = parse_coord_double(reader, false)?;
                Arg::Coords(Coordinates::World([
                    yaw,
                    pitch,
                    Coord {
                        relative: true,
                        value: 0.0,
                    },
                ]))
            }
            ArgType::Angle => {
                if !reader.can_read() {
                    return Err(CommandError::tr_at(
                        "argument.angle.incomplete",
                        &[],
                        reader,
                    ));
                }
                let relative = reader.peek() == '~';
                if relative {
                    reader.skip();
                }
                let value = if reader.can_read() && reader.peek() != ' ' {
                    f64::from(reader.read_float()?)
                } else {
                    0.0
                };
                if value.is_nan() || value.is_infinite() {
                    return Err(CommandError::tr_at("argument.angle.invalid", &[], reader));
                }
                Arg::Angle(Coord { relative, value })
            }
            ArgType::Item => Arg::Item(super::item::parse_stack(reader, reg)?),
            ArgType::ItemPredicate => Arg::Item(super::item::parse_predicate(reader, reg)?),
            ArgType::BlockState => Arg::Block(super::block::parse(reader, reg, false)?),
            ArgType::BlockPredicate => Arg::Block(super::block::parse(reader, reg, true)?),
            ArgType::Resource(kind) => {
                // ResourceArgument: the element must exist.
                let id = read_identifier(reader)?;
                let registry = kind.registry();
                if !reg.ids(registry).contains(&id) {
                    return Err(CommandError::tr_at(
                        "argument.resource.not_found",
                        &[&id, &registry],
                        reader,
                    ));
                }
                Arg::Resource(id)
            }
            ArgType::Res(registry) => {
                let id = read_identifier(reader)?;
                if !reg.ids(registry).contains(&id) {
                    return Err(CommandError::tr_at(
                        "argument.resource.not_found",
                        &[&id, registry],
                        reader,
                    ));
                }
                Arg::Resource(id)
            }
            ArgType::ResKey(_) => Arg::Resource(read_identifier(reader)?),
            ArgType::ResOrTag(registry) | ArgType::ResOrTagKey(registry) => {
                let key_only = matches!(self, ArgType::ResOrTagKey(_));
                if reader.can_read() && reader.peek() == '#' {
                    let cursor = reader.cursor;
                    reader.skip();
                    let id = match read_identifier(reader) {
                        Ok(id) => id,
                        Err(e) => {
                            reader.cursor = cursor;
                            return Err(e);
                        }
                    };
                    if !key_only && !reg.tags(registry).contains(&id) {
                        let e = CommandError::tr_at(
                            "argument.resource_tag.not_found",
                            &[&id, registry],
                            reader,
                        );
                        reader.cursor = cursor;
                        return Err(e);
                    }
                    Arg::Resource(format!("#{id}"))
                } else {
                    let id = read_identifier(reader)?;
                    if !key_only && !reg.ids(registry).contains(&id) {
                        return Err(CommandError::tr_at(
                            "argument.resource.not_found",
                            &[&id, registry],
                            reader,
                        ));
                    }
                    Arg::Resource(id)
                }
            }
            ArgType::StrEnum(values, key) => {
                // StringRepresentableArgument (the error follows the word).
                let name = reader.read_unquoted_string();
                if !values.contains(&name) {
                    return Err(CommandError::tr_at(key, &[&name], reader));
                }
                Arg::Str(name.to_owned())
            }
            ArgType::ResourceSelector(registry) => {
                let start = reader.cursor;
                while reader.can_read() && {
                    let c = reader.peek();
                    is_id_char(c) || c == '*' || c == '?'
                } {
                    reader.skip();
                }
                let raw = reader.string[start..reader.cursor].to_owned();
                let pattern = if raw.contains(':') {
                    raw
                } else {
                    format!("minecraft:{raw}")
                };
                if select_resources(reg.ids(registry), &pattern).is_empty() {
                    return Err(CommandError::tr_at(
                        "argument.resource_selector.not_found",
                        &[&pattern, registry],
                        reader,
                    ));
                }
                Arg::Str(pattern)
            }
            ArgType::HexColor => {
                let text = reader.read_unquoted_string();
                if !matches!(text.len(), 3 | 6) {
                    return Err(CommandError::tr_at(
                        "argument.hexcolor.invalid",
                        &[&text],
                        reader,
                    ));
                }
                // Integer.parseInt(s, begin, end, 16) per channel: a bad digit
                // escapes Brigadier as a dispatcher parse exception.
                let width = text.len() / 3;
                for chunk in text.as_bytes().chunks(width) {
                    let c = std::str::from_utf8(chunk).unwrap_or("");
                    let signed = c.len() > 1 && (c.starts_with('+') || c.starts_with('-'));
                    let digits = if signed { &c[1..] } else { c };
                    if let Some(i) = digits.find(|ch: char| !ch.is_ascii_hexdigit()) {
                        let index = i + usize::from(signed);
                        let message = format!("Error at index {index} in: \"{c}\"");
                        return Err(CommandError::tr_at(
                            "command.exception",
                            &[&message],
                            reader,
                        ));
                    }
                }
                let v = i64::from_str_radix(text, 16).unwrap_or(0);
                let rgb = if text.len() == 3 {
                    let d = |i: u32| ((v >> (8 - 4 * i)) & 15) * 17;
                    (d(0) << 16) | (d(1) << 8) | d(2)
                } else {
                    v
                };
                Arg::Int(rgb as i32)
            }
            ArgType::Uuid => {
                // UuidArgument: the longest run of [-A-Fa-f0-9] must be a UUID.
                let rest = reader.remaining();
                let run: usize = rest
                    .chars()
                    .take_while(|c| c.is_ascii_hexdigit() || *c == '-')
                    .map(char::len_utf8)
                    .sum();
                match (run > 0)
                    .then(|| horizon_core::text::parse_java_uuid(&rest[..run]))
                    .flatten()
                {
                    Some(_) => {
                        let text = rest[..run].to_owned();
                        reader.cursor += run;
                        Arg::Str(text)
                    }
                    None => return Err(CommandError::tr_at("argument.uuid.invalid", &[], reader)),
                }
            }
            ArgType::GameProfile => {
                if reader.can_read() && reader.peek() == '@' {
                    let s = selector::parse(reader, reg, reg.permission >= 2)?;
                    if s.include_entities {
                        return Err(CommandError::tr_at("argument.player.entities", &[], reader));
                    }
                    Arg::Entity(s)
                } else {
                    while reader.can_read() && reader.peek() != ' ' {
                        reader.skip();
                    }
                    Arg::Str(reader.string[start..reader.cursor].to_owned())
                }
            }
            ArgType::Id(_) => Arg::Resource(read_id(reader)?),
            // DimensionArgument: checked when the command runs.
            ArgType::Dimension => Arg::Resource(read_id(reader)?),
            ArgType::Time { min } => {
                let v = reader.read_float()?;
                let unit = reader.read_unquoted_string();
                let Some(m) = time_unit(unit) else {
                    return Err(CommandError::tr_at(
                        "argument.time.invalid_unit",
                        &[],
                        reader,
                    ));
                };
                let ticks = horizon_core::jmath::round_f32(v * m as f32);
                if ticks < *min {
                    return Err(CommandError::tr_at(
                        "argument.time.tick_count_too_low",
                        &[min, &ticks],
                        reader,
                    ));
                }
                Arg::Int(ticks)
            }
            ArgType::GameMode => {
                let name = reader.read_unquoted_string();
                match GameMode::from_name(name) {
                    Some(m) => Arg::GameMode(m),
                    None => {
                        return Err(CommandError::tr_at(
                            "argument.gamemode.invalid",
                            &[&name],
                            reader,
                        ));
                    }
                }
            }
            ArgType::Nbt => {
                super::snbt::parse_compound_argument(reader)?;
                Arg::Str(reader.string[start..reader.cursor].to_owned())
            }
            ArgType::Component | ArgType::Style => {
                // ParserBasedArgument.withCodec: SNBT, then the codec (errors at the start).
                let tag = super::snbt::parse_tag_argument(reader)?;
                let (decoded, key) = if matches!(self, ArgType::Style) {
                    (
                        super::component::decode_style(&tag),
                        "argument.style.invalid",
                    )
                } else {
                    (
                        super::component::decode(&tag, reg),
                        "argument.component.invalid",
                    )
                };
                if let Err(m) = decoded {
                    reader.cursor = start;
                    return Err(CommandError::tr_at(key, &[&m], reader));
                }
                let text = &reader.string[start..reader.cursor];
                match horizon_client_core::component::parse_snbt(text) {
                    Ok((v, _)) => Arg::Str(v.to_string()),
                    Err(_) => Arg::Str(text.to_owned()),
                }
            }
            ArgType::Swizzle => {
                let mut axes = String::new();
                while reader.can_read() && reader.peek() != ' ' {
                    let c = reader.read();
                    if !matches!(c, 'x' | 'y' | 'z') || axes.contains(c) {
                        return Err(CommandError::tr_at(
                            "arguments.swizzle.invalid",
                            &[],
                            reader,
                        ));
                    }
                    axes.push(c);
                }
                Arg::Str(axes)
            }
            ArgType::ScoreHolder { multiple } => {
                if reader.can_read() && reader.peek() == '@' {
                    let sel = selector::parse(reader, reg, reg.permission >= 2)?;
                    if !*multiple && sel.limit > 1 {
                        return Err(CommandError::tr_at("argument.entity.toomany", &[], reader));
                    }
                    Arg::ScoreHolder(ScoreHolderArg::Selector(sel))
                } else {
                    while reader.can_read() && reader.peek() != ' ' {
                        reader.skip();
                    }
                    let text = &reader.string[start..reader.cursor];
                    if text == "*" {
                        Arg::ScoreHolder(ScoreHolderArg::Wildcard)
                    } else {
                        Arg::ScoreHolder(ScoreHolderArg::Name(text.to_owned()))
                    }
                }
            }
            ArgType::Objective | ArgType::Team => {
                Arg::Str(reader.read_unquoted_string().to_owned())
            }
            ArgType::Criteria => {
                while reader.can_read() && reader.peek() != ' ' {
                    reader.skip();
                }
                let id = &reader.string[start..reader.cursor];
                // ObjectiveCriteria.byName: statistics resolve to their canonical name.
                match horizon_client_core::scoreboard::criteria_by_name(id, reg) {
                    Ok(Some(canonical)) => Arg::Str(canonical),
                    Ok(None) => {
                        reader.cursor = start;
                        return Err(CommandError::tr_at(
                            "argument.criteria.invalid",
                            &[&id],
                            reader,
                        ));
                    }
                    Err(_) => {
                        reader.cursor = start;
                        return Err(CommandError::tr_at(
                            "argument.criteria.invalid",
                            &[&id],
                            reader,
                        ));
                    }
                }
            }
            ArgType::ScoreboardSlot => {
                let name = reader.read_unquoted_string();
                if !horizon_client_core::scoreboard::display_slot_valid(name) {
                    return Err(CommandError::tr_at(
                        "argument.scoreboardDisplaySlot.invalid",
                        &[&name],
                        reader,
                    ));
                }
                Arg::Str(name.to_owned())
            }
            ArgType::Operation => {
                if !reader.can_read() {
                    return Err(CommandError::tr_at(
                        "arguments.operation.invalid",
                        &[],
                        reader,
                    ));
                }
                while reader.can_read() && reader.peek() != ' ' {
                    reader.skip();
                }
                let op = &reader.string[start..reader.cursor];
                if !OPERATIONS.contains(&op) {
                    return Err(CommandError::tr("arguments.operation.invalid", &[]));
                }
                Arg::Str(op.to_owned())
            }
            ArgType::TeamColor => {
                let name = reader.read_unquoted_string();
                if horizon_client_core::scoreboard::team_color_rgb(name).is_none() {
                    return Err(CommandError::tr_at(
                        "argument.color.invalid",
                        &[&name],
                        reader,
                    ));
                }
                Arg::Str(name.to_owned())
            }
            ArgType::IntRange => Arg::IntRange(read_int_bounds(reader)?),
            ArgType::FloatRange => Arg::DoubleRange(read_double_bounds(reader)?),
            ArgType::Name(_) => Arg::Str(reader.read_string()?),
            ArgType::Vec2 | ArgType::ColumnPos => {
                let block = matches!(self, ArgType::ColumnPos);
                let one = |reader: &mut Reader| {
                    if block {
                        parse_coord_int(reader)
                    } else {
                        // Vec2Argument.vec2(): integers are centred on the block.
                        parse_coord_double(reader, true)
                    }
                };
                let x = one(reader)?;
                if !reader.can_read() || reader.peek() != ' ' {
                    reader.cursor = start;
                    return Err(CommandError::tr_at(
                        "argument.pos2d.incomplete",
                        &[],
                        reader,
                    ));
                }
                reader.skip();
                let z = one(reader)?;
                Arg::Coords(Coordinates::World([
                    x,
                    Coord {
                        relative: true,
                        value: 0.0,
                    },
                    z,
                ]))
            }
            ArgType::Particle => {
                // ParticleArgument.readParticle: the type, then SNBT options
                // decoded by the type's codec (errors without context).
                let id = read_identifier(reader)?;
                let known = reg.ids("minecraft:particle_type");
                let exists = if known.is_empty() {
                    horizon_client_core::particle_type::ParticleType::from_name(&id).is_some()
                } else {
                    known.contains(&id)
                };
                if !exists {
                    return Err(CommandError::tr_at("particle.notFound", &[&id], reader));
                }
                let options = if reader.can_read() && reader.peek() == '{' {
                    super::snbt::parse_tag_argument(reader)?
                } else {
                    horizon_core::nbt::Tag::Compound(Default::default())
                };
                if let Err(m) = particle_options(&id, &options) {
                    return Err(CommandError::tr("particle.invalidOptions", &[&m]));
                }
                Arg::Str(reader.string[start..reader.cursor].to_owned())
            }
            ArgType::Enum(words) => {
                let word = reader.read_unquoted_string();
                if !words.contains(&word) {
                    reader.cursor = start;
                    return Err(CommandError::tr_at("command.unknown.argument", &[], reader));
                }
                Arg::Str(word.to_owned())
            }
            ArgType::Function => {
                let tag = reader.can_read() && reader.peek() == '#';
                if tag {
                    reader.skip();
                }
                let id = read_id(reader)?;
                Arg::Resource(if tag { format!("#{id}") } else { id })
            }
            ArgType::Slots | ArgType::Slot => {
                // SlotRanges.read (the error is reported after the name).
                while reader.can_read() && reader.peek() != ' ' {
                    reader.skip();
                }
                let word = &reader.string[start..reader.cursor];
                let Some((_, size)) = slot_ranges().into_iter().find(|(n, _)| n == word) else {
                    return Err(CommandError::tr_at("slot.unknown", &[&word], reader));
                };
                if matches!(self, ArgType::Slot) && size != 1 {
                    return Err(CommandError::tr_at(
                        "slot.only_single_allowed",
                        &[&word],
                        reader,
                    ));
                }
                Arg::Str(word.to_owned())
            }
            ArgType::SlotSource => {
                // SlotSourceArgument: a slot range name, else a slot source id or inline value.
                while reader.can_read() && reader.peek() != ' ' {
                    reader.skip();
                }
                let word = &reader.string[start..reader.cursor];
                if slot_ranges().iter().any(|(n, _)| n == word) {
                    Arg::Str(word.to_owned())
                } else {
                    reader.cursor = start;
                    super::codec::resource_or_id(reader, reg, "minecraft:slot_source")?;
                    Arg::Str(reader.string[start..reader.cursor].to_owned())
                }
            }
            ArgType::ResourceOrId(registry) => {
                super::codec::resource_or_id(reader, reg, registry)?;
                Arg::Resource(reader.string[start..reader.cursor].to_owned())
            }
            ArgType::NbtPath => {
                parse_nbt_path(reader)?;
                Arg::Str(reader.string[start..reader.cursor].to_owned())
            }
            ArgType::NbtTag => {
                super::snbt::parse_tag_argument(reader)?;
                Arg::Str(reader.string[start..reader.cursor].to_owned())
            }
            ArgType::Anchor => {
                let name = reader.read_unquoted_string();
                match name {
                    "eyes" => Arg::Anchor(true),
                    "feet" => Arg::Anchor(false),
                    _ => {
                        reader.cursor = start;
                        return Err(CommandError::tr_at(
                            "argument.anchor.invalid",
                            &[&name],
                            reader,
                        ));
                    }
                }
            }
            ArgType::Custom(custom) => {
                let text = match custom.kind {
                    CustomArgumentKind::Word => reader.read_unquoted_string().to_owned(),
                    CustomArgumentKind::Greedy => {
                        let text = reader.remaining().to_owned();
                        reader.cursor = reader.string.len();
                        text
                    }
                };
                match (custom.parse)(&text) {
                    Ok(value) => Arg::Custom {
                        kind: custom.id.clone(),
                        value,
                    },
                    Err(message) => {
                        reader.cursor = start;
                        return Err(CommandError::at(message, reader));
                    }
                }
            }
        })
    }
    pub fn suggest(
        &self,
        builder: &mut SuggestionsBuilder,
        reg: &Registries,
        context: &Context,
        _name: &str,
    ) {
        let _ = context;
        match self {
            ArgType::Bool => builder.suggest_matching(["true", "false"]),
            ArgType::Entity { players, .. } => {
                // EntityArgument.listSuggestions: the online player names,
                // then (unless players only) getSelectedEntities.
                let mut names = reg.players.clone();
                if !players {
                    names.extend(reg.selected_entities.iter().cloned());
                }
                selector::suggest(builder, reg, &|b| {
                    b.suggest_matching(names.iter().map(String::as_str))
                });
            }
            ArgType::BlockPos => suggest_coordinates(builder, reg, self, true),
            ArgType::Vec3 { .. } => suggest_coordinates(builder, reg, self, false),
            // RotationArgument and AngleArgument have no suggestions.
            ArgType::Rotation | ArgType::Angle => {}
            ArgType::Item => super::item::suggest_stack(builder, reg),
            ArgType::ItemPredicate => super::item::suggest_predicate(builder, reg),
            ArgType::BlockState => super::block::suggest(builder, reg, false),
            ArgType::BlockPredicate => super::block::suggest(builder, reg, true),
            ArgType::Resource(kind) => suggest_resources(builder, reg.ids(kind.registry()), ""),
            ArgType::Res(registry) | ArgType::ResKey(registry) => {
                // RecipeCommand filters its keys to learnable recipes.
                if *registry == "minecraft:recipe" && !reg.learnable_recipes.is_empty() {
                    suggest_resources(builder, &reg.learnable_recipes, "")
                } else {
                    suggest_resources(builder, reg.ids(registry), "")
                }
            }
            ArgType::ResOrTag(registry) | ArgType::ResOrTagKey(registry) => {
                suggest_resources(builder, reg.tags(registry), "#");
                suggest_resources(builder, reg.ids(registry), "");
            }
            ArgType::StrEnum(values, _) => builder.suggest_matching(values.iter().copied()),
            ArgType::HexColor => builder.suggest_matching(["F00", "FF0000"]),
            ArgType::ResourceSelector(registry) => {
                suggest_resources(builder, reg.ids(registry), "")
            }
            ArgType::GameProfile => {
                let players = reg.players.clone();
                selector::suggest(builder, reg, &|b| {
                    b.suggest_matching(players.iter().map(String::as_str))
                });
            }
            ArgType::Id(list) => {
                let values = match *list {
                    "storages" => reg.storages.clone(),
                    "bossbars" => reg.bossbars.clone(),
                    "schedules" => reg.schedules.clone(),
                    "stopwatches" => reg.stopwatches.clone(),
                    "recipes" => reg.recipes.clone(),
                    "sounds" => reg.sounds.clone(),
                    "damage_types" => reg.damage_types.clone(),
                    "attributes" => reg.attributes.clone(),
                    "loot_tables" => reg.loot_tables.clone(),
                    "templates" => reg.templates.clone(),
                    "random_sequences" => reg.random_sequences.clone(),
                    "predicates" => reg.predicates.clone(),
                    "item_modifiers" => reg.item_modifiers.clone(),
                    "time_markers" => reg.time_markers.clone(),
                    "advancements" => reg.advancements.clone(),
                    _ => Vec::new(),
                };
                suggest_resources(builder, &values, "");
            }
            ArgType::Dimension => {
                let dims = reg.dimensions.clone();
                suggest_resources(builder, &dims, "");
            }
            ArgType::Time { .. } => {
                let mut r = Reader::new(builder.remaining());
                if r.read_float().is_ok() {
                    let mut b = builder.offset(builder.start + r.cursor);
                    b.suggest_matching(["d", "s", "t", ""]);
                    builder.out.extend(b.out);
                }
            }
            ArgType::GameMode => builder.suggest_matching(GameMode::ALL.map(GameMode::name)),
            ArgType::ScoreHolder { .. } => {
                // ScoreHolderArgument.SUGGEST_SCORE_HOLDERS.
                let players = reg.players.clone();
                selector::suggest(builder, reg, &|b| {
                    b.suggest_matching(players.iter().map(String::as_str))
                });
            }
            ArgType::Objective => {
                builder.suggest_matching(reg.objectives.iter().map(String::as_str))
            }
            ArgType::Team => builder.suggest_matching(reg.teams.iter().map(String::as_str)),
            ArgType::Criteria => {
                let mut ids: Vec<String> = horizon_client_core::scoreboard::CUSTOM_CRITERIA
                    .iter()
                    .map(|s| (*s).to_owned())
                    .collect();
                for c in horizon_client_core::scoreboard::team_colors() {
                    ids.push(format!("teamkill.{c}"));
                    ids.push(format!("killedByTeam.{c}"));
                }
                let stat =
                    |ty: &str, key: &str| horizon_client_core::scoreboard::stat_criteria(ty, key);
                ids.extend(reg.blocks.keys().map(|b| stat("mined", b)));
                for ty in ["crafted", "used", "broken", "picked_up", "dropped"] {
                    ids.extend(reg.items.iter().map(|i| stat(ty, i)));
                }
                for ty in ["killed", "killed_by"] {
                    ids.extend(reg.entity_types.iter().map(|e| stat(ty, e)));
                }
                ids.extend(
                    horizon_client_core::scoreboard::CUSTOM_STATS
                        .iter()
                        .map(|c| stat("custom", &format!("minecraft:{c}"))),
                );
                builder.suggest_matching(ids.iter().map(String::as_str));
            }
            ArgType::ScoreboardSlot => {
                let slots = horizon_client_core::scoreboard::display_slots();
                builder.suggest_matching(slots.iter().map(String::as_str));
            }
            ArgType::Operation => builder.suggest_matching(OPERATIONS),
            ArgType::Enum(words) => builder.suggest_matching(words.iter().copied()),
            ArgType::Particle => {
                let known = reg.ids("minecraft:particle_type");
                if known.is_empty() {
                    let names: Vec<String> = horizon_client_core::particle_type::ParticleType::ALL
                        .iter()
                        .map(|t| format!("minecraft:{}", t.name()))
                        .collect();
                    suggest_resources(builder, &names, "");
                } else {
                    suggest_resources(builder, known, "");
                }
            }
            ArgType::Vec2 => suggest_coordinates(builder, reg, self, false),
            ArgType::ColumnPos => suggest_coordinates(builder, reg, self, true),
            ArgType::Name(list) => {
                let names = match *list {
                    "datapacks_enabled" => reg.datapacks_enabled.clone(),
                    _ => reg.datapacks_disabled.clone(),
                };
                builder.suggest_matching(names.iter().map(String::as_str));
            }
            // FunctionArgument has no suggestions of its own (nodes add SUGGEST_FUNCTION).
            ArgType::Function => {}
            ArgType::Slots | ArgType::Slot => {
                let names: Vec<String> = if matches!(self, ArgType::Slots) {
                    slot_ranges().into_iter().map(|(n, _)| n).collect()
                } else {
                    slot_names()
                };
                builder.suggest_matching(names.iter().map(String::as_str));
            }
            ArgType::SlotSource => {
                let mut sub = builder.offset(builder.start);
                let names: Vec<String> = slot_ranges().into_iter().map(|(n, _)| n).collect();
                sub.suggest_matching(names.iter().map(String::as_str));
                builder.add(sub);
                suggest_resources(builder, reg.ids("minecraft:slot_source"), "");
            }
            ArgType::ResourceOrId(registry) => suggest_resources(builder, reg.ids(registry), ""),
            ArgType::TeamColor => {
                builder.suggest_matching(horizon_client_core::scoreboard::team_colors())
            }
            ArgType::Anchor => builder.suggest_matching(["eyes", "feet"]),
            ArgType::Component | ArgType::Style | ArgType::NbtTag => super::snbt::suggest(builder),
            ArgType::Swizzle | ArgType::Message | ArgType::Nbt => {}
            ArgType::Custom(custom) => {
                let prefix = builder.remaining().to_owned();
                for suggestion in (custom.suggest)(&prefix) {
                    match suggestion.tooltip {
                        Some(tooltip) => builder.suggest_tooltip(suggestion.text, tooltip),
                        None => builder.suggest(suggestion.text),
                    }
                }
            }
            _ => {}
        }
    }
}
