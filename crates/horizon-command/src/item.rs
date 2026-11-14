//! ItemParser (item stacks with components, `stone[damage=5]`) and
//! ItemPredicateArgument (ComponentPredicateParser's packrat grammar:
//! `#logs[count~1..|!damage]`), with vanilla's errors and suggestions.
use super::args::{
    ItemInput, Registries, is_id_char, parse_identifier, read_identifier, suggest_resources,
};
use super::codec::{either, is_number, not_map, snbt};
use super::packrat::{Grammar, Reason, State, Sugg, Term, Value, alt, ch, opt, seq};
use super::{CmdResult, CommandError, Reader, SuggestionsBuilder};
use horizon_core::nbt::{Compound, Tag};
use horizon_world::datafix::dynamic::as_number;
use std::rc::Rc;

fn int_of(t: &Tag) -> Option<i64> {
    Some(match t {
        Tag::Byte(v) => i64::from(*v),
        Tag::Short(v) => i64::from(*v),
        Tag::Int(v) => i64::from(*v),
        Tag::Long(v) => *v as i32 as i64,
        Tag::Float(v) => f32::from_bits(*v) as i32 as i64,
        Tag::Double(v) => f64::from_bits(*v) as i32 as i64,
        _ => return None,
    })
}
/// Built-ins with an implemented shared component payload decoder.
pub const DECODED_COMPONENTS: &[&str] = &[
    "minecraft:damage",
    "minecraft:repair_cost",
    "minecraft:max_damage",
    "minecraft:max_stack_size",
    "minecraft:map_id",
    "minecraft:rarity",
    "minecraft:enchantments",
    "minecraft:stored_enchantments",
    "minecraft:enchantment_glint_override",
    "minecraft:custom_name",
    "minecraft:item_name",
    "minecraft:item_model",
    "minecraft:block_entity_data",
    "minecraft:enchantable",
    "minecraft:ominous_bottle_amplifier",
    "minecraft:potion_duration_scale",
    "minecraft:minimum_attack_charge",
    "minecraft:dyed_color",
];

/// Validate and decode a component into the value representation shared with stacks.
/// Registered components without a shared decoder retain their exact stored value.
/// Registration is checked by the caller; this helper only rejects failed decodes.
pub fn decoded_component_value(id: &str, value: &Tag, reg: &Registries) -> Result<Tag, String> {
    if !DECODED_COMPONENTS.contains(&id) {
        return Ok(value.clone());
    }
    if id == "minecraft:potion_duration_scale" {
        return potion_duration_scale(value).map(|value| Tag::Float(value.to_bits()));
    }
    if id == "minecraft:item_model" {
        return item_model(value);
    }
    decode_component(id, value, reg)?;
    if id == "minecraft:block_entity_data" {
        let Tag::Compound(_) = value else {
            return Err(not_map(value));
        };
        if !matches!(value.get("id"), Some(Tag::String(_))) {
            return Err("block entity id is not a string".into());
        }
    }
    Ok(match id {
        "minecraft:damage"
        | "minecraft:repair_cost"
        | "minecraft:max_damage"
        | "minecraft:max_stack_size"
        | "minecraft:map_id"
        | "minecraft:ominous_bottle_amplifier" => {
            Tag::Int(int_of(value).expect("validated numeric component") as i32)
        }
        "minecraft:enchantable" => {
            let value = value.get("value").expect("validated enchantable value");
            Tag::compound([(
                "value",
                Tag::Int(int_of(value).expect("validated numeric enchantable value") as i32),
            )])
        }
        "minecraft:minimum_attack_charge" => Tag::Float(
            as_number(value)
                .expect("validated numeric component")
                .float_value()
                .to_bits(),
        ),
        "minecraft:enchantment_glint_override" => Tag::Byte(i8::from(
            horizon_items::item_data::component_bool(value).expect("validated boolean component"),
        )),
        "minecraft:dyed_color" => {
            Tag::Int(horizon_core::color::rgb_color_from_tag(value).expect("validated RGB color"))
        }
        "minecraft:enchantments" | "minecraft:stored_enchantments" => {
            let Tag::Compound(entries) = value else {
                unreachable!("validated enchantment map")
            };
            Tag::Compound(
                entries
                    .iter()
                    .map(|(id, level)| {
                        let id = parse_identifier(&id.to_string_lossy())
                            .expect("validated enchantment id");
                        (
                            id.as_str().into(),
                            Tag::Int(int_of(level).expect("validated level") as i32),
                        )
                    })
                    .collect(),
            )
        }
        _ => value.clone(),
    })
}

/// The codecs of the item components commands commonly set.
pub fn decode_component(id: &str, t: &Tag, reg: &Registries) -> Result<(), String> {
    match id {
        // MapId.CODEC is Codec.INT, without a non-negative constraint.
        "minecraft:map_id" => int_of(t).map(|_| ()).ok_or_else(|| "Not a number".into()),
        "minecraft:dyed_color" => horizon_core::color::rgb_color_from_tag(t)
            .map(|_| ())
            .ok_or_else(|| "Expected a number or a list of three numeric RGB channels".into()),
        "minecraft:damage" | "minecraft:repair_cost" => {
            let v = int_of(t).ok_or("Not a number")?;
            if v < 0 {
                return Err(format!("Value must be non-negative: {v}"));
            }
            Ok(())
        }
        "minecraft:max_damage" => {
            let v = int_of(t).ok_or("Not a number")?;
            if v <= 0 {
                return Err(format!("Value must be positive: {v}"));
            }
            Ok(())
        }
        "minecraft:max_stack_size" => {
            let v = int_of(t).ok_or("Not a number")?;
            if !(1..=99).contains(&v) {
                return Err(format!("Value must be within range [1;99]: {v}"));
            }
            Ok(())
        }
        // Enchantable.CODEC: a record with one required positive integer.
        "minecraft:enchantable" => {
            let Tag::Compound(_) = t else {
                return Err(not_map(t));
            };
            let value = t
                .get("value")
                .ok_or_else(|| format!("No key value in {}", snbt(t)))?;
            let v = int_of(value).ok_or("Not a number")?;
            if v <= 0 {
                return Err(format!("Value must be positive: {v}"));
            }
            Ok(())
        }
        // The amplifier integer codec accepts only 0 through 4, without clamping.
        "minecraft:ominous_bottle_amplifier" => {
            let v = int_of(t).ok_or("Not a number")?;
            if !(0..=4).contains(&v) {
                return Err(format!("Value must be within range [0;4]: {v}"));
            }
            Ok(())
        }
        "minecraft:potion_duration_scale" => potion_duration_scale(t).map(|_| ()),
        // DataComponents.MINIMUM_ATTACK_CHARGE / ExtraCodecs.floatRange:
        // validate Number.floatValue with Float.compareTo, which rejects -0.0 and NaN.
        "minecraft:minimum_attack_charge" => {
            let value = as_number(t).ok_or("Not a number")?.float_value();
            if value.is_sign_negative() || !(0.0..=1.0).contains(&value) {
                return Err(format!(
                    "Value must be within range [0.0;1.0]: {}",
                    horizon_core::java_number::java_float(value)
                ));
            }
            Ok(())
        }
        "minecraft:rarity" => match t {
            Tag::String(s) => {
                let s = s.to_string_lossy();
                if ["common", "uncommon", "rare", "epic"].contains(&s.as_str()) {
                    Ok(())
                } else {
                    Err(format!("Unknown element name:{s}"))
                }
            }
            _ => Err("Not a string".into()),
        },
        "minecraft:enchantments" | "minecraft:stored_enchantments" => enchantment_map(t, reg),
        // DataComponents.ENCHANTMENT_GLINT_OVERRIDE: Codec.BOOL through NbtOps.
        "minecraft:enchantment_glint_override" => horizon_items::item_data::component_bool(t)
            .map(|_| ())
            .ok_or_else(|| "Not a number".into()),
        "minecraft:custom_name" | "minecraft:item_name" => super::component::decode(t),
        "minecraft:item_model" => item_model(t).map(|_| ()),
        // TypedEntityData.codec: the type is the compound's id.
        "minecraft:block_entity_data" => match t {
            Tag::Compound(c) => match c.get(&"id".into()) {
                None => Err(format!("Expected 'id' field in {}", snbt(t))),
                Some(Tag::String(id)) => {
                    let id = id.to_string_lossy();
                    let id = if id.contains(':') {
                        id
                    } else {
                        format!("minecraft:{id}")
                    };
                    if horizon_client_core::block_entity_nbt::is_known_type(&id) {
                        Ok(())
                    } else {
                        Err(format!(
                            "Unknown registry key in ResourceKey[minecraft:root / minecraft:block_entity_type]: {id}"
                        ))
                    }
                }
                Some(_) => Ok(()),
            },
            _ => Ok(()),
        },
        _ => Ok(()),
    }
}

/// `ExtraCodecs.NON_NEGATIVE_FLOAT`: boxed `Number.floatValue`, then `Float.compareTo`.
/// The inclusive bounds reject negative zero, NaN, and both infinities.
fn potion_duration_scale(t: &Tag) -> Result<f32, String> {
    let value = match t {
        Tag::Byte(value) => f32::from(*value),
        Tag::Short(value) => f32::from(*value),
        Tag::Int(value) => *value as f32,
        Tag::Long(value) => *value as f32,
        Tag::Float(value) => f32::from_bits(*value),
        Tag::Double(value) => f64::from_bits(*value) as f32,
        _ => return Err("Not a number".into()),
    };
    if value.is_finite() && !value.is_sign_negative() {
        Ok(value)
    } else {
        Err(format!("Value must be non-negative: {value}"))
    }
}
/// `DataComponents.ITEM_MODEL`: `Identifier.CODEC`, without a resource lookup.
fn item_model(value: &Tag) -> Result<Tag, String> {
    let Tag::String(value) = value else {
        return Err("Not a string".into());
    };
    let text =
        String::from_utf16(&value.0).map_err(|_| "Not a valid resource location".to_owned())?;
    let id = horizon_core::identifier::Identifier::parse(&text)
        .map_err(|error| format!("Not a valid resource location: {text} {error}"))?;
    Ok(Tag::String(id.to_string().as_str().into()))
}
/// `ItemEnchantments.CODEC`: registry keys and boxed-number levels in 1..=255.
/// `UnboundedMapCodec` reports failed entries; commands do not retain its partial map.
fn enchantment_map(t: &Tag, reg: &Registries) -> Result<(), String> {
    let Tag::Compound(map) = t else {
        return Err(not_map(t));
    };
    let mut errors = Vec::new();
    let mut missed = Compound::new();
    for (key, value) in map {
        let mut entry_errors = Vec::new();
        match int_of(value) {
            None => entry_errors.push("Not a number".to_owned()),
            Some(level) if !(1..=255).contains(&level) => {
                entry_errors.push(format!("Value {level} outside of range [1:255]"));
            }
            _ => {}
        }
        let raw = key.to_string_lossy();
        match parse_identifier(&raw) {
            Some(id) if reg.ids("minecraft:enchantment").contains(&id) => {}
            Some(id) => entry_errors.push(format!(
                "Failed to get element {id} from registry minecraft:enchantment"
            )),
            None => entry_errors.push(format!("Not a valid resource location: {raw}")),
        }
        if !entry_errors.is_empty() {
            errors.extend(entry_errors);
            missed.insert(key.clone(), value.clone());
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} missed input: {}",
            errors.join("; "),
            snbt(&Tag::Compound(missed))
        ))
    }
}

/// MinMaxBounds.Ints.CODEC.
fn int_bounds(t: &Tag) -> Result<(), String> {
    match t {
        Tag::Compound(_) => Ok(()),
        _ if is_number(t) => Ok(()),
        _ => Err(either(not_map(t), "Not a number".into())),
    }
}
/// DataComponentPredicate type codecs.
pub fn decode_predicate(id: &str, t: &Tag) -> Result<(), String> {
    match id {
        "minecraft:count" => int_bounds(t),
        "minecraft:damage" => match t {
            Tag::Compound(_) => Ok(()),
            _ => Err(not_map(t)),
        },
        _ => Ok(()),
    }
}

// ---- ItemParser -------------------------------------------------------

#[derive(Clone, Copy, Debug)]
enum ISugg {
    Nothing,
    Item,
    StartComponents,
    ComponentAssignmentOrRemoval,
    Component,
    Assignment,
    NextOrEndComponents,
}

struct ItemState<'r, 'a> {
    reader: &'r mut Reader<'a>,
    reg: &'r Registries,
    sugg: ISugg,
}
impl ItemState<'_, '_> {
    fn parse(&mut self) -> CmdResult<ItemInput> {
        self.sugg = ISugg::Item;
        let cursor = self.reader.cursor;
        let id = read_identifier(self.reader)?;
        if !self.reg.ids("minecraft:item").contains(&id) {
            self.reader.cursor = cursor;
            return Err(CommandError::tr_at(
                "argument.item.id.invalid",
                &[&id],
                self.reader,
            ));
        }
        self.sugg = ISugg::StartComponents;
        let mut components = None;
        if self.reader.can_read() && self.reader.peek() == '[' {
            self.sugg = ISugg::Nothing;
            let start = self.reader.cursor;
            self.read_components()?;
            components = Some(self.reader.string[start..self.reader.cursor].to_owned());
        }
        Ok(ItemInput {
            id,
            tag: false,
            any: false,
            components,
        })
    }
    fn component_type(&mut self) -> CmdResult<String> {
        if !self.reader.can_read() {
            return Err(CommandError::tr_at(
                "arguments.item.component.expected",
                &[],
                self.reader,
            ));
        }
        let cursor = self.reader.cursor;
        let id = read_identifier(self.reader)?;
        if self.reg.persistent_components.contains(&id) {
            return Ok(id);
        }
        self.reader.cursor = cursor;
        Err(CommandError::tr_at(
            "arguments.item.component.unknown",
            &[&id],
            self.reader,
        ))
    }
    fn read_components(&mut self) -> CmdResult<()> {
        self.reader.expect('[')?;
        self.sugg = ISugg::ComponentAssignmentOrRemoval;
        let mut visited: Vec<String> = Vec::new();
        while self.reader.can_read() && self.reader.peek() != ']' {
            self.reader.skip_whitespace();
            if self.reader.can_read() && self.reader.peek() == '!' {
                self.reader.skip();
                self.sugg = ISugg::Component;
                let ty = self.component_type()?;
                if visited.contains(&ty) {
                    return Err(CommandError::tr(
                        "arguments.item.component.repeated",
                        &[&ty],
                    ));
                }
                visited.push(ty);
                self.sugg = ISugg::Nothing;
                self.reader.skip_whitespace();
            } else {
                let ty = self.component_type()?;
                if visited.contains(&ty) {
                    return Err(CommandError::tr(
                        "arguments.item.component.repeated",
                        &[&ty],
                    ));
                }
                visited.push(ty.clone());
                self.sugg = ISugg::Assignment;
                self.reader.skip_whitespace();
                self.reader.expect('=')?;
                self.sugg = ISugg::Nothing;
                self.reader.skip_whitespace();
                let cursor = self.reader.cursor;
                let tag = super::snbt::parse_tag_argument(self.reader)?;
                if let Err(m) = decode_component(&ty, &tag, self.reg) {
                    self.reader.cursor = cursor;
                    return Err(CommandError::tr_at(
                        "arguments.item.component.malformed",
                        &[&ty, &m],
                        self.reader,
                    ));
                }
                self.reader.skip_whitespace();
            }
            self.sugg = ISugg::NextOrEndComponents;
            if !self.reader.can_read() || self.reader.peek() != ',' {
                break;
            }
            self.reader.skip();
            self.reader.skip_whitespace();
            self.sugg = ISugg::ComponentAssignmentOrRemoval;
            if !self.reader.can_read() {
                return Err(CommandError::tr_at(
                    "arguments.item.component.expected",
                    &[],
                    self.reader,
                ));
            }
        }
        self.reader.expect(']')?;
        self.sugg = ISugg::Nothing;
        Ok(())
    }
    fn suggest_component(&self, b: &mut SuggestionsBuilder, suffix: &str) {
        let contents = b.remaining_lower();
        for id in &self.reg.persistent_components {
            let matched = if contents.contains(':') {
                super::args::matches_sub_str(&contents, id)
            } else {
                let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
                super::args::matches_sub_str(&contents, ns)
                    || super::args::matches_sub_str(&contents, path)
            };
            if matched {
                b.suggest(format!("{id}{suffix}"));
            }
        }
    }
    fn fill(&self, builder: &mut SuggestionsBuilder) {
        let mut b = builder.offset(self.reader.cursor);
        let empty = b.remaining().is_empty();
        match self.sugg {
            ISugg::Nothing => {}
            ISugg::Item => suggest_resources(&mut b, self.reg.ids("minecraft:item"), ""),
            ISugg::StartComponents => {
                if empty {
                    b.suggest("[");
                }
            }
            ISugg::ComponentAssignmentOrRemoval => {
                b.suggest("!");
                self.suggest_component(&mut b, "=");
            }
            ISugg::Component => self.suggest_component(&mut b, ""),
            ISugg::Assignment => {
                if empty {
                    b.suggest("=");
                }
            }
            ISugg::NextOrEndComponents => {
                if empty {
                    b.suggest(",");
                    b.suggest("]");
                }
            }
        }
        builder.add(b);
    }
}

/// ItemArgument.parse (ItemParser.parse; the reader is rewound on errors).
pub fn parse_stack(reader: &mut Reader, reg: &Registries) -> CmdResult<ItemInput> {
    let cursor = reader.cursor;
    let r = ItemState {
        reader,
        reg,
        sugg: ISugg::Nothing,
    }
    .parse();
    if r.is_err() {
        reader.cursor = cursor;
    }
    r
}
/// ItemParser.fillSuggestions.
pub fn suggest_stack(builder: &mut SuggestionsBuilder, reg: &Registries) {
    let mut reader = Reader {
        string: builder.input,
        cursor: builder.start,
    };
    let mut state = ItemState {
        reader: &mut reader,
        reg,
        sugg: ISugg::Nothing,
    };
    let _ = state.parse();
    state.fill(builder);
}

// ---- ItemPredicateArgument ------------------------------------------------

#[derive(Clone, Debug)]
pub enum TypeRes {
    Item(String),
    Tag(String),
}
#[derive(Clone, Debug)]
pub enum IV {
    Id(String),
    Type(Option<TypeRes>),
    Elem(TypeRes),
    Unit,
    Test,
    Comp(String),
    Pred(String),
    Tag(Tag),
    List(Vec<IV>),
}
impl Value for IV {
    fn list(items: Vec<Self>) -> Self {
        IV::List(items)
    }
}

fn reg<'a>(state: &State<'a, IV>) -> &'a Registries {
    state
        .ctx
        .and_then(|c| c.downcast_ref::<Registries>())
        .expect("item predicate grammar needs the registries")
}

/// IdentifierParseRule: skip whitespace, a non-empty identifier (no error).
fn id_rule(state: &mut State<IV>) -> Option<IV> {
    let mut r = state.reader();
    r.skip_whitespace();
    let start = r.cursor;
    while r.can_read() && is_id_char(r.peek()) {
        r.skip();
    }
    let raw = &r.string[start..r.cursor];
    let id = if raw.is_empty() {
        None
    } else {
        parse_identifier(raw)
    };
    match id {
        Some(id) => {
            state.set_reader(&r);
            Some(IV::Id(id))
        }
        None => {
            r.cursor = start;
            state.set_reader(&r);
            None
        }
    }
}

/// ResourceLookupRule: an identifier validated by `validate`.
fn lookup(
    state: &mut State<IV>,
    id_rule: usize,
    possible: Rc<Vec<String>>,
    validate: &dyn Fn(&str, &Reader) -> Result<IV, CommandError>,
) -> Option<IV> {
    let mut r = state.reader();
    r.skip_whitespace();
    state.set_reader(&r);
    let mark = state.cursor;
    match state.parse(id_rule) {
        Some(IV::Id(id)) => match validate(&id, &state.reader()) {
            Ok(v) => Some(v),
            Err(e) => {
                state.store(mark, Sugg::Resources(possible), Reason::Error(e));
                None
            }
        },
        _ => {
            state.store(
                mark,
                Sugg::Resources(possible),
                Reason::Delayed("argument.id.invalid", Vec::new()),
            );
            None
        }
    }
}

fn predicate_grammar() -> Grammar<IV> {
    let mut g: Grammar<IV> = Grammar::new();
    let id = g.put_custom("id", id_rule);
    let ty = g.named("type");
    let conditions = g.named("conditions");
    let ty2 = g.named("type");
    g.put_term(
        "top",
        alt(vec![
            seq(vec![ty, ch('['), Term::Cut, opt(conditions), ch(']')]),
            ty2,
        ]),
        |s| {
            let mut out = Vec::new();
            if let Some(t) = s.scope.get("type") {
                out.push(t);
            }
            if let Some(IV::List(c)) = s.scope.get("conditions") {
                out.extend(c);
            }
            Some(IV::List(out))
        },
    );
    let element = g.named("element_type");
    let tag = g.named("tag_type");
    let any = g.named("any_type");
    g.put_term(
        "type",
        alt(vec![element, seq(vec![ch('#'), Term::Cut, tag]), any]),
        |s| {
            Some(IV::Type(
                match s.scope.get_any(&["element_type", "tag_type"]) {
                    Some(IV::Elem(t)) => Some(t),
                    _ => None,
                },
            ))
        },
    );
    g.put_term("any_type", ch('*'), |_| Some(IV::Unit));
    g.put_custom("element_type", move |s| {
        let r = reg(s);
        let items = Rc::new(r.ids("minecraft:item").to_vec());
        lookup(s, id, items, &|id, reader| {
            if r.ids("minecraft:item").iter().any(|i| i == id) {
                Ok(IV::Elem(TypeRes::Item(id.to_owned())))
            } else {
                Err(CommandError::tr_at(
                    "argument.item.id.invalid",
                    &[&id],
                    reader,
                ))
            }
        })
    });
    g.put_custom("tag_type", move |s| {
        let r = reg(s);
        let tags = Rc::new(r.tags("minecraft:item").to_vec());
        lookup(s, id, tags, &|id, reader| {
            if r.tags("minecraft:item").iter().any(|i| i == id) {
                Ok(IV::Elem(TypeRes::Tag(id.to_owned())))
            } else {
                Err(CommandError::tr_at(
                    "arguments.item.tag.unknown",
                    &[&id],
                    reader,
                ))
            }
        })
    });
    let alternatives = g.named("alternatives");
    let rest = g.named("conditions");
    g.put_term(
        "conditions",
        seq(vec![alternatives, opt(seq(vec![ch(','), rest]))]),
        |s| {
            let mut out = vec![IV::Test];
            if let Some(IV::List(c)) = s.scope.get("conditions") {
                out.extend(c);
            }
            Some(IV::List(out))
        },
    );
    let term = g.named("term");
    let rest = g.named("alternatives");
    g.put_term(
        "alternatives",
        seq(vec![term, opt(seq(vec![ch('|'), rest]))]),
        |s| {
            let mut out = vec![s.scope.get("term").unwrap_or(IV::Test)];
            if let Some(IV::List(c)) = s.scope.get("alternatives") {
                out.extend(c);
            }
            Some(IV::List(out))
        },
    );
    let test = g.named("test");
    let negation = g.named("negation");
    g.put_term("term", alt(vec![test, seq(vec![ch('!'), negation])]), |s| {
        s.scope.get_any(&["test", "negation"])
    });
    let test = g.named("test");
    g.put_term("negation", test, |_| Some(IV::Test));
    let comp = g.named("component_type");
    let tag_a = g.named("tag");
    let pred = g.named("predicate_type");
    let tag_b = g.named("tag");
    let comp2 = g.named("component_type");
    g.put_term(
        "test",
        alt(vec![
            seq(vec![comp, ch('='), Term::Cut, tag_a]),
            seq(vec![pred, ch('~'), Term::Cut, tag_b]),
            comp2,
        ]),
        |s| {
            let value = match s.scope.get("tag") {
                Some(IV::Tag(t)) => Some(t),
                _ => None,
            };
            let reader = s.reader();
            let result = if let Some(IV::Pred(p)) = s.scope.get("predicate_type") {
                decode_predicate(&p, value.as_ref().unwrap_or(&Tag::End)).map_err(|m| {
                    CommandError::tr_at("arguments.item.predicate.malformed", &[&p, &m], &reader)
                })
            } else if let (Some(IV::Comp(c)), Some(v)) = (s.scope.get("component_type"), value) {
                decode_component(&c, &v, reg(s)).map_err(|m| {
                    CommandError::tr_at("arguments.item.component.malformed", &[&c, &m], &reader)
                })
            } else {
                Ok(())
            };
            match result {
                Ok(()) => Some(IV::Test),
                Err(e) => {
                    s.store(s.cursor, Sugg::Empty, Reason::Error(e));
                    None
                }
            }
        },
    );
    g.put_custom("component_type", move |s| {
        let r = reg(s);
        let mut list = vec!["minecraft:count".to_owned()];
        list.extend(r.persistent_components.iter().cloned());
        lookup(s, id, Rc::new(list), &|id, reader| {
            if id == "minecraft:count" {
                return Ok(IV::Comp(id.to_owned()));
            }
            let known = r
                .ids("minecraft:data_component_type")
                .iter()
                .any(|c| c == id);
            if known && r.persistent_components.iter().any(|c| c == id) {
                Ok(IV::Comp(id.to_owned()))
            } else {
                Err(CommandError::tr_at(
                    "arguments.item.component.unknown",
                    &[&id],
                    reader,
                ))
            }
        })
    });
    g.put_custom("predicate_type", move |s| {
        let r = reg(s);
        let mut list = vec!["minecraft:count".to_owned()];
        list.extend(
            r.ids("minecraft:data_component_predicate_type")
                .iter()
                .cloned(),
        );
        lookup(s, id, Rc::new(list), &|id, reader| {
            if id == "minecraft:count"
                || r.ids("minecraft:data_component_predicate_type")
                    .iter()
                    .any(|c| c == id)
            {
                return Ok(IV::Pred(id.to_owned()));
            }
            if r.ids("minecraft:data_component_type")
                .iter()
                .any(|c| c == id)
            {
                // A component existence predicate (decodes Unit).
                return Ok(IV::Pred(format!("exists:{id}")));
            }
            Err(CommandError::tr_at(
                "arguments.item.predicate.unknown",
                &[&id],
                reader,
            ))
        })
    });
    g.put_custom("tag", |s| super::snbt::rule_in(s, IV::Tag));
    g.top = g.id("top");
    g
}

thread_local! {
    static PREDICATE: Grammar<IV> = predicate_grammar();
}
/// ItemPredicateArgument.parse.
pub fn parse_predicate(reader: &mut Reader, reg: &Registries) -> CmdResult<ItemInput> {
    let start = reader.cursor;
    let result = PREDICATE.with(|g| g.parse_for_commands_with(reader, Some(reg)))?;
    let IV::List(items) = result else {
        return Err(CommandError::tr_at("argument.id.invalid", &[], reader));
    };
    let text = &reader.string[start..reader.cursor];
    let (id, tag, any) = match items.first() {
        Some(IV::Type(Some(TypeRes::Item(i)))) => (i.clone(), false, false),
        Some(IV::Type(Some(TypeRes::Tag(t)))) => (t.clone(), true, false),
        _ => ("*".to_owned(), false, true),
    };
    let components = text.find('[').map(|i| text[i..].to_owned());
    Ok(ItemInput {
        id,
        tag,
        any,
        components,
    })
}
/// ItemPredicateArgument.listSuggestions.
pub fn suggest_predicate(builder: &mut SuggestionsBuilder, reg: &Registries) {
    PREDICATE.with(|g| g.parse_for_suggestions_with(builder, Some(reg)));
}

#[cfg(test)]
mod tests {
    use super::decode_component;

    #[test]
    fn decoded_component_values_use_codec_numbers_and_retain_stored_values() {
        use super::{Registries, Tag, decoded_component_value};
        let registry = Registries::default();
        assert_eq!(
            decoded_component_value(
                "minecraft:damage",
                &Tag::Double(5.0f64.to_bits()),
                &registry
            )
            .unwrap(),
            Tag::Int(5)
        );
        assert_eq!(
            decoded_component_value(
                "minecraft:repair_cost",
                &Tag::Float(2.0f32.to_bits()),
                &registry
            )
            .unwrap(),
            Tag::Int(2)
        );
        assert!(
            decoded_component_value(
                "minecraft:damage",
                &Tag::String("not a number".into()),
                &registry
            )
            .is_err()
        );
        assert_eq!(
            decoded_component_value(
                "minecraft:unbreakable",
                &Tag::compound([] as [(&str, Tag); 0]),
                &registry
            )
            .unwrap(),
            Tag::compound([] as [(&str, Tag); 0])
        );
        assert!(
            decoded_component_value(
                "minecraft:block_entity_data",
                &Tag::String("invalid".into()),
                &registry
            )
            .is_err()
        );
    }

    #[test]
    fn enchantment_maps_validate_entries_against_the_supplied_registry() {
        use super::{Reader, Registries, parse_predicate, parse_stack};
        use crate::args::RegistryIds;
        let mut reg = Registries {
            items: vec!["minecraft:diamond_sword".into()],
            persistent_components: vec![
                "minecraft:enchantments".into(),
                "minecraft:stored_enchantments".into(),
            ],
            ..Default::default()
        };
        reg.by_registry.insert(
            "minecraft:data_component_type".into(),
            RegistryIds {
                ids: reg.persistent_components.clone(),
                tags: vec![],
            },
        );
        reg.by_registry.insert(
            "minecraft:enchantment".into(),
            RegistryIds {
                ids: vec!["minecraft:sharpness".into()],
                tags: vec![],
            },
        );
        for component in ["enchantments", "stored_enchantments"] {
            for value in [
                "{}",
                "{sharpness:1}",
                "{sharpness:255}",
                "{sharpness:1.5f}",
                "{sharpness:1b}",
                "{sharpness:4294967297L}",
                "{sharpness:9007199254740993L}",
                "{\"minecraft:sharpness\":1}",
            ] {
                let input = format!("diamond_sword[{component}={value}]");
                for parser in [parse_stack, parse_predicate] {
                    let mut reader = Reader::new(&input);
                    assert!(parser(&mut reader, &reg).is_ok(), "{input}");
                    assert_eq!(reader.cursor, input.len());
                }
            }
            for value in [
                "{sharpness:-1}",
                "{sharpness:0}",
                "{sharpness:256}",
                "{sharpness:100000}",
                "{sharpness:-0.5d}",
                "{sharpness:\"1\"}",
                "{levels:{sharpness:1}}",
                "{\"audit:missing\":1}",
            ] {
                let input = format!("diamond_sword[{component}={value}]");
                for parser in [parse_stack, parse_predicate] {
                    assert!(
                        parser(&mut Reader::new(&input), &reg).is_err(),
                        "{input} crossed command parsing"
                    );
                }
            }
        }
    }

    fn check(snbt: &str) -> Result<(), String> {
        let tag = horizon_core::nbt::parse_snbt(snbt).expect("snbt").0;
        decode_component(
            "minecraft:block_entity_data",
            &tag,
            &super::Registries::default(),
        )
    }

    #[test]
    fn block_entity_data_needs_a_known_type() {
        assert_eq!(check("{id:\"chest\",Lock:1}"), Ok(()));
        assert_eq!(
            check("{Command:\"say\"}"),
            Err("Expected 'id' field in {Command:\"say\"}".into())
        );
        assert_eq!(
            check("{id:\"minecraft:stone\"}"),
            Err("Unknown registry key in ResourceKey[minecraft:root / minecraft:block_entity_type]: minecraft:stone".into())
        );
    }

    #[test]
    #[ignore = "daily"]
    fn block_entity_data_takes_a_type_a_mod_declared() {
        // The declared types are frozen once per process: install them in a fresh one.
        const CHILD: &str = "HORIZON_TEST_DECLARED_BLOCK_ENTITY_DATA";
        if std::env::var_os(CHILD).is_none() {
            assert!(
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--include-ignored",
                        "--exact",
                        "item::tests::block_entity_data_takes_a_type_a_mod_declared",
                        "--nocapture"
                    ])
                    .env(CHILD, "1")
                    .status()
                    .unwrap()
                    .success()
            );
            return;
        }
        use horizon_client_core::block_entity_types::{self, BlockEntityTypes};
        block_entity_types::install(BlockEntityTypes::new(
            [("example:crate".into(), "example:crate".into())].into(),
            vec!["example:crate".into()],
        ))
        .unwrap();
        assert_eq!(check("{id:\"example:crate\",Tank:1}"), Ok(()));
        assert_eq!(
            check("{id:\"example:nope\"}"),
            Err("Unknown registry key in ResourceKey[minecraft:root / minecraft:block_entity_type]: example:nope".into())
        );
    }
}
