// Included in mod_content.rs to retain its private scope.
fn face_rules_json(rules: &[mod_api::FaceRule]) -> J {
    let state_obj = |map: &BTreeMap<String, String>| -> J {
        J::Obj(
            map.iter()
                .map(|(k, v)| (k.clone(), J::Str(v.clone())))
                .collect(),
        )
    };
    let dir_name = |d: sunrise_core::geometry::Direction| match d {
        sunrise_core::geometry::Direction::Down => "down",
        sunrise_core::geometry::Direction::Up => "up",
        sunrise_core::geometry::Direction::North => "north",
        sunrise_core::geometry::Direction::South => "south",
        sunrise_core::geometry::Direction::West => "west",
        sunrise_core::geometry::Direction::East => "east",
    };
    J::Arr(
        rules
            .iter()
            .map(|rule| {
                let mut fields = Vec::new();
                match &rule.neighbor {
                    mod_api::FaceNeighbor::Any => {}
                    mod_api::FaceNeighbor::SameBlock { state } => {
                        let val = if state.is_empty() {
                            J::Str("self".into())
                        } else {
                            J::Obj(vec![
                                ("block".into(), J::Str("self".into())),
                                ("state".into(), state_obj(state)),
                            ])
                        };
                        fields.push(("neighbor".into(), val));
                    }
                    mod_api::FaceNeighbor::Block { id, state } => {
                        let val = if state.is_empty() {
                            J::Str(id.clone())
                        } else {
                            J::Obj(vec![
                                ("block".into(), J::Str(id.clone())),
                                ("state".into(), state_obj(state)),
                            ])
                        };
                        fields.push(("neighbor".into(), val));
                    }
                    mod_api::FaceNeighbor::Tag { id, state } => {
                        let val = if state.is_empty() {
                            J::Str(format!("#{id}"))
                        } else {
                            J::Obj(vec![
                                ("tag".into(), J::Str(id.clone())),
                                ("state".into(), state_obj(state)),
                            ])
                        };
                        fields.push(("neighbor".into(), val));
                    }
                }
                if !rule.state.is_empty() {
                    fields.push(("state".into(), state_obj(&rule.state)));
                }
                if !rule.same.is_empty() {
                    fields.push((
                        "same".into(),
                        J::Arr(rule.same.iter().cloned().map(J::Str).collect()),
                    ));
                }
                match &rule.faces {
                    mod_api::FaceDirections::All => {}
                    mod_api::FaceDirections::Horizontal => {
                        fields.push(("faces".into(), J::Str("horizontal".into())));
                    }
                    mod_api::FaceDirections::Vertical => {
                        fields.push(("faces".into(), J::Str("vertical".into())));
                    }
                    mod_api::FaceDirections::Only(dirs) => {
                        fields.push((
                            "faces".into(),
                            J::Arr(dirs.iter().map(|&d| J::Str(dir_name(d).into())).collect()),
                        ));
                    }
                }
                if !rule.facts.is_empty() {
                    let mut facts = Vec::new();
                    if let Some(v) = rule.facts.is_air {
                        facts.push(("is_air".into(), J::Bool(v)));
                    }
                    if let Some(v) = rule.facts.can_occlude {
                        facts.push(("can_occlude".into(), J::Bool(v)));
                    }
                    if let Some(v) = rule.facts.is_solid_render {
                        facts.push(("is_solid_render".into(), J::Bool(v)));
                    }
                    if let Some(v) = rule.facts.liquid {
                        facts.push(("liquid".into(), J::Bool(v)));
                    }
                    fields.push(("facts".into(), J::Obj(facts)));
                }
                J::Obj(fields)
            })
            .collect(),
    )
}

/// JSON with its object keys in file order, so a rewritten table differs
/// from the original only by what was added.
#[derive(Clone, Debug, PartialEq)]
enum J {
    Null,
    Bool(bool),
    Num(serde_json::Number),
    Str(String),
    Arr(Vec<J>),
    Obj(Vec<(String, J)>),
}
impl<'de> Deserialize<'de> for J {
    fn deserialize<D: de::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct V;
        impl<'de> de::Visitor<'de> for V {
            type Value = J;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("JSON")
            }
            fn visit_unit<E>(self) -> std::result::Result<J, E> {
                Ok(J::Null)
            }
            fn visit_bool<E>(self, v: bool) -> std::result::Result<J, E> {
                Ok(J::Bool(v))
            }
            fn visit_i64<E>(self, v: i64) -> std::result::Result<J, E> {
                Ok(J::Num(v.into()))
            }
            fn visit_u64<E>(self, v: u64) -> std::result::Result<J, E> {
                Ok(J::Num(v.into()))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> std::result::Result<J, E> {
                serde_json::Number::from_f64(v)
                    .map(J::Num)
                    .ok_or_else(|| E::custom("not a JSON number"))
            }
            fn visit_str<E>(self, v: &str) -> std::result::Result<J, E> {
                Ok(J::Str(v.to_owned()))
            }
            fn visit_string<E>(self, v: String) -> std::result::Result<J, E> {
                Ok(J::Str(v))
            }
            fn visit_seq<A: de::SeqAccess<'de>>(
                self,
                mut a: A,
            ) -> std::result::Result<J, A::Error> {
                let mut out = Vec::new();
                while let Some(v) = a.next_element()? {
                    out.push(v);
                }
                Ok(J::Arr(out))
            }
            fn visit_map<A: de::MapAccess<'de>>(
                self,
                mut a: A,
            ) -> std::result::Result<J, A::Error> {
                let mut out = Vec::new();
                while let Some(kv) = a.next_entry()? {
                    out.push(kv);
                }
                Ok(J::Obj(out))
            }
        }
        d.deserialize_any(V)
    }
}
impl Serialize for J {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::{SerializeMap, SerializeSeq};
        match self {
            J::Null => s.serialize_unit(),
            J::Bool(b) => s.serialize_bool(*b),
            J::Num(n) => n.serialize(s),
            J::Str(t) => s.serialize_str(t),
            J::Arr(a) => {
                let mut seq = s.serialize_seq(Some(a.len()))?;
                for v in a {
                    seq.serialize_element(v)?;
                }
                seq.end()
            }
            J::Obj(o) => {
                let mut map = s.serialize_map(Some(o.len()))?;
                for (k, v) in o {
                    map.serialize_entry(k, v)?;
                }
                map.end()
            }
        }
    }
}
impl J {
    fn get(&self, key: &str) -> Option<&J> {
        match self {
            J::Obj(o) => o.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    fn remove(&mut self, key: &str) {
        if let J::Obj(o) = self {
            o.retain(|(k, _)| k != key);
        }
    }
    fn get_mut(&mut self, key: &str) -> Option<&mut J> {
        match self {
            J::Obj(o) => o.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    /// Sets `key`, in place if present, else at the end.
    fn set(&mut self, key: &str, value: J) {
        if let Some(v) = self.get_mut(key) {
            *v = value;
        } else if let J::Obj(o) = self {
            o.push((key.to_owned(), value));
        }
    }
    /// The value at a `/`-separated path ("" is the document).
    fn at(&mut self, path: &str) -> Option<&mut J> {
        path.split('/')
            .filter(|p| !p.is_empty())
            .try_fold(self, |j, key| j.get_mut(key))
    }
    fn f64(v: f64) -> J {
        serde_json::Number::from_f64(v).map_or(J::Null, J::Num)
    }
    fn as_u32(&self) -> Option<u32> {
        match self {
            J::Num(n) => n.as_u64().and_then(|v| u32::try_from(v).ok()),
            _ => None,
        }
    }
    fn as_str(&self) -> Option<&str> {
        match self {
            J::Str(s) => Some(s),
            _ => None,
        }
    }
    /// Renames strings and, with `ids`, moves state ids, everywhere inside.
    fn rename(&mut self, r: &Rename, ids: bool) {
        match self {
            J::Str(s) => {
                if let Some(new) = r.string(s) {
                    *s = new;
                }
            }
            J::Num(n) if ids => {
                if let Some(new) = n
                    .as_u64()
                    .and_then(|v| u32::try_from(v).ok())
                    .and_then(|id| r.id(id))
                {
                    *n = new.into();
                }
            }
            J::Arr(a) => a.iter_mut().for_each(|v| v.rename(r, ids)),
            J::Obj(o) => o.iter_mut().for_each(|(_, v)| v.rename(r, ids)),
            _ => {}
        }
    }
}

/// One id moving to another: `minecraft:iron_block` → `ruby:ruby_block`,
/// with its state range for blocks.
#[derive(Clone)]
struct Rename {
    from: String,
    to: String,
    states: Range<u32>,
    first: u32,
    /// States this block gives each of its template's: one per combination
    /// of the state properties it declares (1 when it declares none). A
    /// template state moves to the first of them, which is the one every
    /// declared property is at its default for.
    fanout: u32,
    /// The lookup from a template state and a declared combination to the state's row, as an
    /// offset from `first`: `rows[template offset * fanout + combination]`, laid out as vanilla
    /// does (`state_order`). Empty: the template's states times the combinations, in that order.
    rows: Vec<u32>,
}
impl Rename {
    fn new(from: &str, to: &str) -> Self {
        Rename {
            from: from.to_owned(),
            to: to.to_owned(),
            states: 0..0,
            first: 0,
            fanout: 1,
            rows: Vec::new(),
        }
    }
    /// The id itself, `Block{id}`, and `block.<ns>.<path>` / `item.<ns>.<path>` keys.
    fn string(&self, s: &str) -> Option<String> {
        if s == self.from {
            return Some(self.to.clone());
        }
        if s.strip_prefix("Block{").and_then(|x| x.strip_suffix('}')) == Some(&self.from) {
            return Some(format!("Block{{{}}}", self.to));
        }
        let key = |id: &str| id.replacen(':', ".", 1).replace('/', ".");
        for kind in ["block", "item"] {
            if s.strip_prefix(kind).and_then(|x| x.strip_prefix('.')) == Some(&key(&self.from)) {
                return Some(format!("{kind}.{}", key(&self.to)));
            }
        }
        None
    }
    /// The row of a template state's `combination`, as an offset from `first`.
    fn row(&self, at: u32, combination: u32) -> u32 {
        let index = at * self.fanout + combination;
        self.rows.get(index as usize).copied().unwrap_or(index)
    }
    /// Where a template state moves: its combination with every declared property at its default.
    fn id(&self, id: u32) -> Option<u32> {
        self.states
            .contains(&id)
            .then(|| self.first + self.row(id - self.states.start, 0))
    }
}

/// A template's own state properties, sorted by name, each with how many values it takes across
/// the template's `states`: how `StateTable::new` reads them back, the first name slowest.
fn template_properties<'a>(
    states: &Range<u32>,
    properties: impl Fn(u32) -> Option<&'a J>,
) -> Vec<(String, u32)> {
    let Some(J::Obj(own)) = states.clone().next().and_then(&properties) else {
        return Vec::new();
    };
    let mut names: Vec<&str> = own.iter().map(|(k, _)| k.as_str()).collect();
    names.sort_unstable();
    let mut out = Vec::new();
    for name in names {
        let mut seen: Vec<&J> = Vec::new();
        for id in states.clone() {
            if let Some(v) = properties(id).and_then(|p| p.get(name))
                && !seen.contains(&v)
            {
                seen.push(v);
            }
        }
        out.push((name.to_owned(), (seen.len() as u32).max(1)));
    }
    out
}

/// A block's states: in vanilla's order when the template's own properties multiply out to its
/// states (every vanilla block's do), else the template's states times the combinations.
fn block_order<'a>(
    states: &Range<u32>,
    properties: impl Fn(u32) -> Option<&'a J>,
    declared: &[(String, u32)],
    combinations: u32,
) -> Vec<(u32, u32)> {
    let template = template_properties(states, properties);
    if template.iter().map(|p| p.1).product::<u32>() == states.len() as u32 {
        return state_order(&template, declared);
    }
    (0..states.len() as u32)
        .flat_map(|state| (0..combinations).map(move |combo| (state, combo)))
        .collect()
}

/// The states of a block in vanilla's order (`StateDefinition`): every property, the template's
/// and the declared ones, sorted by name, the first name varying slowest. Each entry is a
/// template state's offset and the index of a declared combination (`combinations`), whose own
/// order is by declared name, the first slowest. With every declared name after the template's
/// this is the template's states times the combinations, as before.
fn state_order(template: &[(String, u32)], declared: &[(String, u32)]) -> Vec<(u32, u32)> {
    // (name, values, its own side's stride, from the template)
    let mut all: Vec<(&str, u32, u32, bool)> = Vec::new();
    for (side, from_template) in [(template, true), (declared, false)] {
        let mut stride = 1;
        let mut strides = Vec::new();
        for (_, len) in side.iter().rev() {
            strides.push(stride);
            stride *= len;
        }
        for ((name, len), stride) in side.iter().zip(strides.into_iter().rev()) {
            all.push((name, *len, stride, from_template));
        }
    }
    all.sort_by(|a, b| a.0.cmp(b.0));
    let total: u32 = all.iter().map(|p| p.1).product();
    (0..total)
        .map(|position| {
            let (mut state, mut combo, mut rest) = (0, 0, total);
            for &(_, len, stride, from_template) in &all {
                rest /= len;
                let value = (position / rest) % len;
                if from_template {
                    state += value * stride;
                } else {
                    combo += value * stride;
                }
            }
            (state, combo)
        })
        .collect()
}

/// The combinations of a block's declared state properties, in a fixed
/// order: by name (a `BTreeMap`'s), each one's declared default first, and
/// the first name varying slowest. So the first combination is every
/// property at its default, which is the one a template state maps to.
fn combinations(
    declared: &BTreeMap<String, StateProperty>,
    default_state: &BTreeMap<String, String>,
) -> Result<Vec<Vec<(String, String)>>> {
    let mut orders: Vec<(&str, Vec<&str>)> = Vec::new();
    for (name, property) in declared {
        let mut values = property.values();
        if let Some(default) = default_state.get(name) {
            let Some(at) = values.iter().position(|v| v == default) else {
                return Err(
                    format!("{name}: default_state {default:?} is not one of its values").into(),
                );
            };
            values.rotate_left(at);
        }
        orders.push((name, values));
    }
    // Every combination is a state of each of the template's, so a block's run
    // is its template's states times this many: refused before it is built.
    let mut product: u64 = 1;
    for (_, values) in &orders {
        product = product.saturating_mul(values.len() as u64);
    }
    if product > MAX_STATES as u64 {
        return Err(format!(
            "{product} state property combinations, more than the {MAX_STATES} a world can hold"
        )
        .into());
    }
    let mut combos: Vec<Vec<(String, String)>> = vec![Vec::new()];
    for (name, values) in orders {
        let mut next = Vec::with_capacity(combos.len().saturating_mul(values.len()));
        for combo in &combos {
            for value in &values {
                let mut c = combo.clone();
                c.push((name.to_owned(), (*value).to_owned()));
                next.push(c);
            }
        }
        combos = next;
    }
    Ok(combos)
}

/// `flags` with its legacy solid bit (bit 0) calculated from `shape`, unless the template forces it.
fn with_solid_bit(flags: i64, force: Option<bool>, shape: &VoxelShape) -> i64 {
    let definition = BlockDefinitionProperties {
        force_solid_on: force == Some(true),
        force_solid_off: force == Some(false),
        replaceable: false,
        liquid: false,
        can_occlude: false,
    };
    (flags & !1) | i64::from(definition.calculate_solid(Some(shape)))
}

/// Puts a declared state property's values on one of a block's state rows.
/// Only `facts.json` keeps a state's own row, whose fifth field is
/// `{"id": …, "properties": {…}}`; the other files' rows are per block, so
/// they have no property map to extend.
fn set_state_properties(row: &mut J, combo: &[(String, String)]) {
    let J::Arr(parts) = row else {
        return;
    };
    let Some(J::Obj(state)) = parts.get_mut(4) else {
        return;
    };
    if !state.iter().any(|(k, _)| k == "properties") {
        state.push(("properties".into(), J::Obj(Vec::new())));
    }
    let Some((_, properties)) = state.iter_mut().find(|(k, _)| k == "properties") else {
        return;
    };
    for (name, value) in combo {
        properties.set(name, J::Str(value.clone()));
    }
}

/// The same for an NBT table's state, which spells a state the way
/// `BlockState::to_nbt` does: `{id: "…", properties: {"<name>": "<value>"}}`,
/// with the compound left out when there are no properties.
fn set_nbt_properties(state: &mut Tag, combo: &[(String, String)]) {
    if combo.is_empty() {
        return;
    }
    let Tag::Compound(c) = state else {
        return;
    };
    let properties = c
        .entry(JavaString::from("properties"))
        .or_insert_with(|| Tag::Compound(Compound::new()));
    let Tag::Compound(properties) = properties else {
        return;
    };
    for (name, value) in combo {
        properties.insert(
            JavaString::from(name.as_str()),
            Tag::String(JavaString::from(value.as_str())),
        );
    }
}

// DiscreteVoxelShape.forAllEdges: merge consecutive boundary edges.
fn shape_edges(shape: &VoxelShape) -> Vec<J> {
    shape
        .edges()
        .into_iter()
        .map(|edge| J::Arr(edge.into_iter().map(J::f64).collect()))
        .collect()
}

fn encode_shape(shape: &VoxelShape) -> Tag {
    let mut c = nbt::Compound::new();
    let (min, max) = shape.discrete_bounds();
    c.insert("canonical".into(), Tag::Byte(0));
    c.insert("cube".into(), Tag::Byte(i8::from(shape.is_cube())));
    let axes = shape.cube_coordinate_axes();
    c.insert(
        "cube_axes".into(),
        Tag::Byte((axes[0] as i8) | ((axes[1] as i8) << 1) | ((axes[2] as i8) << 2)),
    );
    for (name, axis) in ["x", "y", "z"].into_iter().zip(Axis::ALL) {
        c.insert(
            name.into(),
            Tag::LongArray(
                shape
                    .coordinates(axis)
                    .iter()
                    .map(|n| n.to_bits() as i64)
                    .collect(),
            ),
        );
    }
    let [x, y, z] = shape.sizes();
    let mut occupied = Vec::with_capacity(x * y * z);
    for i in 0..x {
        for j in 0..y {
            for k in 0..z {
                occupied.push(u8::from(shape.is_full(i as i32, j as i32, k as i32)));
            }
        }
    }
    c.insert("occupied".into(), Tag::ByteArray(occupied));
    c.insert("discrete_min".into(), Tag::IntArray(min.to_vec()));
    c.insert("discrete_max".into(), Tag::IntArray(max.to_vec()));
    Tag::Compound(c)
}

/// A state's value of property `name`: its declared combination's, else its template state's.
fn state_value<'a>(
    combo: &'a [(String, String)],
    template: Option<&'a J>,
    name: &str,
) -> Option<&'a str> {
    combo
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
        .or_else(|| match template?.get(name) {
            Some(J::Str(value)) => Some(value.as_str()),
            _ => None,
        })
}

fn resolve_state_values<'a, 's, T>(
    spec: &'s StateValues<T>,
    block: &str,
    states: &Range<u32>,
    order: &[(u32, u32)],
    combos: &[Vec<(String, String)>],
    properties: impl Fn(u32) -> Option<&'a J>,
) -> Result<Vec<&'s T>> {
    let names = spec.property_names();
    let mut used = BTreeSet::new();
    let mut selected = Vec::with_capacity(order.len());
    for &(state, combo) in order {
        let values = names
            .iter()
            .map(|name| {
                state_value(
                    &combos[combo as usize],
                    properties(states.start + state),
                    name,
                )
                .ok_or_else(|| format!("{block}: state lacks property {name}"))
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let key = values.join(",");
        let value = if let Some(value) = spec.values.get(&key) {
            used.insert(key);
            value
        } else {
            spec.default
                .as_ref()
                .ok_or_else(|| format!("{block}: no state value for {key}"))?
        };
        selected.push(value);
    }
    if used.len() != spec.values.len() {
        return Err(
            format!("{block}: state values include an unknown property value tuple").into(),
        );
    }
    Ok(selected)
}

fn resolve_shapes<'a>(
    spec: &BlockShapes,
    block: &str,
    states: &Range<u32>,
    order: &[(u32, u32)],
    combos: &[Vec<(String, String)>],
    properties: impl Fn(u32) -> Option<&'a J>,
) -> Result<Vec<VoxelShape>> {
    resolve_state_values(spec, block, states, order, combos, properties)?
        .into_iter()
        .map(|boxes| {
            let mut shape = VoxelShape::empty();
            for bounds in boxes {
                let part = VoxelShape::from_box(Aabb::new(
                    Vec3::new(bounds[0], bounds[1], bounds[2]),
                    Vec3::new(bounds[3], bounds[4], bounds[5]),
                ))?;
                shape = shape.join_unoptimized(&part, ShapeOp::OR)?;
            }
            Ok(shape)
        })
        .collect()
}

/// A content farmland declaration names a block absent from the loaded registry.
#[derive(Debug)]
pub struct UnknownBaseBlock {
    /// The farmland declaration.
    pub block: String,
    /// The missing base ID.
    pub base: String,
}
impl std::fmt::Display for UnknownBaseBlock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: base_block {} is not a block", self.block, self.base)
    }
}
impl std::error::Error for UnknownBaseBlock {}

/// A block ID resolved against the complete content and vanilla registry.
struct KnownBlock<'a>(&'a str);

/// The template class's rule for downward skylight through a declared outline.
#[derive(Clone, Copy)]
enum SkylightRule {
    Shape,
    Fluid,
    Template,
}

/// A block being added: its declaration and where its states go.
struct NewBlock<'a> {
    stew_effects: Option<J>,
    decl: &'a Block,
    base_block: Option<KnownBlock<'a>>,
    rename: Rename,
    /// The template's item, if it has one.
    item: Option<String>,
    shapes: Option<Vec<VoxelShape>>,
    collision_shapes: Option<Vec<VoxelShape>>,
    /// The template's `isAir` property, which precedes the fluid check in the walk evaluator.
    is_air: bool,
    /// Whether each shaped state contains fluid, including intrinsic template fluids.
    has_fluid: Vec<bool>,
    /// Resolved per-state limits; template inheritance is resolved once at load.
    offset_limits: Vec<Option<(f32, f32)>>,
    skylight: SkylightRule,
    custom_dampening: bool,
    /// Fullness of the published occlusion shape when explicitly enabling occlusion.
    template_occlusion_full: Vec<bool>,
    /// The template's `forceSolidOn` (`Some(true)`) or `forceSolidOff` (`Some(false)`), which keeps
    /// its answer over the declared shape's.
    force_solid: Option<bool>,
    /// The template's light emission per state in registry order, read where a declared
    /// `spawn_rule` of `default` needs it and the block declares no light of its own.
    template_emission: Vec<u8>,
    context_shapes: BTreeMap<String, Vec<VoxelShape>>,
    state_light: Option<Vec<u8>>,
    default: u32,
    /// The declared state properties' combinations, every property at its
    /// default first (one empty combination when the block declares none):
    /// the block's states are its template's states times these.
    combos: Vec<Vec<(String, String)>>,
    /// Each state's template state offset and combination index, in registry order
    /// (`state_order`).
    order: Vec<(u32, u32)>,
    /// The sign whose item places this block on a wall, which makes that item
    /// this block's too (`StandingAndWallBlockItem.registerBlocks`).
    wall_of: Option<String>,
}
impl NewBlock<'_> {
    /// The item its gameplay row names when that is not the template's item
    /// renamed: `block_item`, or `minecraft:air` for a block with no item
    /// (a sign's wall form keeps the paired sign's). `Block.asItem` and, unless
    /// the block has a `pick_item` row of its own, pick-block read this value.
    fn item_row(&self) -> Option<&str> {
        match &self.decl.block_item {
            Some(item) => Some(item),
            None => (!self.decl.item && self.wall_of.is_none()).then_some("minecraft:air"),
        }
    }

    /// Vanilla caches occlusion using the outline at BlockPos.ZERO, not the block-local outline.
    fn declared_occlusion(&self, index: usize, shape: &VoxelShape) -> VoxelShape {
        self.offset_limits[index].map_or_else(
            || shape.clone(),
            |limits| {
                let [x, y, z] = sunrise_core::random::column_offset(limits, 0, 0);
                shape.moved(Vec3::new(x, y, z))
            },
        )
    }

    /// Template states and declared combinations in registry order.
    fn state_combinations(&self) -> impl Iterator<Item = (u32, &[(String, String)])> {
        let start = self.rename.states.start;
        self.order
            .iter()
            .map(move |&(state, combo)| (start + state, self.combos[combo as usize].as_slice()))
    }
}
/// An item being added (declared, or a block's).
struct NewItem {
    rename: Rename,
    max_stack_size: Option<u8>,
    rarity: Option<Rarity>,
    components: Vec<(String, J)>,
    crafting_remainder: Option<String>,
    block: Option<String>,
    arm_pose: Option<sunrise_mod_package::content::ArmPose>,
    /// `StandingAndWallBlockItem.wallBlock`, when its block declares it.
    wall_block: Option<String>,
    template: Option<String>,
}

struct Plan<'a> {
    blocks: Vec<NewBlock<'a>>,
    items: Vec<NewItem>,
    /// Every vanilla item id, including items omitted from the search tab.
    vanilla_items: BTreeSet<String>,
    effects: Vec<&'a MobEffect>,
    fluids: Vec<&'a FluidLook>,
    entity_types: Vec<&'a EntityType>,
    block_entity_types: Vec<&'a BlockEntityType>,
    block_entity_factories: BTreeMap<&'a str, &'a str>,
    menu_types: Vec<&'a MenuType>,
    /// Vanilla's state count (the tables' length before).
    states: usize,
    /// Each vanilla block's default state.
    defaults: BTreeMap<String, u32>,
    /// The gameplay rows' sound objects, by block, for `sounds` overrides.
    sounds: BTreeMap<String, J>,
    /// The mods' creative tabs, in mod order.
    tabs: Vec<&'a CreativeTab>,
    /// The mods' English names of their items and blocks (`item.<ns>.<path>`,
    /// `block.<ns>.<path>`), for the search tab.
    names: BTreeMap<String, String>,
}

/// Component types present in the vanilla item table.
pub fn known_components(base: &Path) -> Result<BTreeSet<String>> {
    let path = base.join("items.json");
    if !path.exists() {
        return Ok(BTreeSet::new());
    }
    Ok(component_types(&read_json(&path)?))
}

fn component_types(map: &J) -> BTreeSet<String> {
    match map {
        J::Obj(entries) => entries
            .iter()
            .filter_map(|(_, v)| match v {
                J::Obj(c) => Some(c.iter().map(|(k, _)| k.clone())),
                _ => None,
            })
            .flatten()
            .collect(),
        _ => BTreeSet::new(),
    }
}

/// A native package's staged declarations. Its dispatch owner is the map key,
/// regardless of the namespaces used in these entries.
#[derive(Clone, Default)]
pub struct DeclaredContent {
    pub blocks: Vec<LocalDeclaration<Block>>,
    pub items: Vec<LocalDeclaration<Item>>,
    pub types: Vec<BlockEntityType>,
    pub fluids: Vec<FluidLook>,
}

/// Join code declarations to their manifest owner, even when no files were admitted.
/// Validate the independent checks before deciding whether to add the owner's batch.
pub fn accept_declared(
    mods: &mut Vec<ContentMod>,
    declared: &BTreeMap<&'static str, DeclaredContent>,
    manifests: &[&sunrise_mod_package::Manifest],
    known: &BTreeSet<String>,
    errors: &mut Vec<DeclarationRefusal>,
) -> Result<()> {
    let mut namespace_error = None;
    for (&id, batch) in declared {
        let already_refused = errors.iter().any(|refusal| refusal.mod_id == id);
        let before = errors.len();
        let existing = mods.iter().position(|m| m.id == id);
        let mut content = Content::declared(id);
        if let Some(manifest) = manifests.iter().find(|m| m.id == id) {
            content.namespaces = manifest.owned_namespaces().map(str::to_owned).collect();
        }
        if let Some(i) = existing {
            content.blocks.clone_from(&mods[i].content.blocks);
            content.items.clone_from(&mods[i].content.items);
        }
        if let Err(refusals) = content.declare_validated(&batch.blocks, &batch.items) {
            errors.extend(
                refusals
                    .into_iter()
                    .map(|(kind, error)| DeclarationRefusal::new(id, kind, error)),
            );
        }
        for block in &batch.blocks {
            let block_id = block.get().id.as_str();
            if let Some(other) = mods.iter().find(|m| {
                m.id != id
                    && (m.content.blocks.iter().any(|b| b.id == block_id)
                        || (block.get().item && m.content.items.iter().any(|i| i.id == block_id)))
            }) {
                errors.push(DeclarationRefusal::new(
                    id,
                    DeclarationKind::Block,
                    DeclarationError::Rejected {
                        id: block_id.to_owned(),
                        reason: format!("declared twice (already declared by mod {})", other.id),
                    },
                ));
            }
        }
        for item in &batch.items {
            let item_id = item.get().id.as_str();
            if let Some(other) = mods.iter().find(|m| {
                m.id != id
                    && (m.content.items.iter().any(|i| i.id == item_id)
                        || m.content.blocks.iter().any(|b| b.item && b.id == item_id))
            }) {
                errors.push(DeclarationRefusal::new(
                    id,
                    DeclarationKind::Item,
                    DeclarationError::Rejected {
                        id: item_id.to_owned(),
                        reason: format!(
                            "declared twice, or is also a block's item (already declared by mod {})",
                            other.id
                        ),
                    },
                ));
            }
        }
        for (kind, name, components) in batch
            .items
            .iter()
            .map(|i| i.get())
            .map(|i| (DeclarationKind::Item, &i.id, &i.components))
            .chain(
                batch
                    .blocks
                    .iter()
                    .map(|b| b.get())
                    .filter(|b| b.item)
                    .map(|b| (DeclarationKind::Block, &b.id, &b.components)),
            )
        {
            if let Err(reason) = validate_components_for(components, known) {
                errors.push(DeclarationRefusal::new(
                    id,
                    kind,
                    DeclarationError::Rejected {
                        id: name.clone(),
                        reason: reason.to_string(),
                    },
                ));
            }
        }
        if already_refused || errors.len() != before {
            continue;
        }
        let target = if let Some(i) = existing {
            let target = &mut mods[i].content;
            target.blocks = content.blocks;
            target.items = content.items;
            target
        } else {
            if let Some(error) = mods.iter().find_map(|m| {
                sunrise_mod_package::content::clash(
                    (&m.id, &m.content.namespaces),
                    (id, &content.namespaces),
                )
            }) {
                namespace_error.get_or_insert(error);
                continue;
            }
            mods.push(ContentMod {
                id: id.into(),
                content,
                jars: vec![],
            });
            &mut mods.last_mut().unwrap().content
        };
        target.block_entity_types.extend_from_slice(&batch.types);
        // A file's fluid look still says how the code-declared fluid looks.
        for fluid in &batch.fluids {
            match target.fluids.iter_mut().find(|f| f.id == fluid.id) {
                Some(look) => {
                    look.world.clone_from(&fluid.world);
                    if fluid.bucket_item.is_some() {
                        look.bucket_item.clone_from(&fluid.bucket_item);
                    }
                }
                None => target.fluids.push(fluid.clone()),
            }
        }
    }
    match namespace_error {
        Some(error) => Err(error.into()),
        None => Ok(()),
    }
}

/// Check and accept one mod without disturbing previously accepted mods.
pub fn accept_mod(
    mods: &mut Vec<ContentMod>,
    candidate: ContentMod,
    known: &BTreeSet<String>,
) -> Result<()> {
    validate_components(&candidate.content, known)?;
    let own = (
        candidate.id.as_str(),
        candidate.content.namespaces.as_slice(),
    );
    if let Some(error) = mods
        .iter()
        .find_map(|c| sunrise_mod_package::content::clash((&c.id, &c.content.namespaces), own))
    {
        return Err(error.into());
    }
    mods.push(candidate);
    Ok(())
}

fn validate_components(
    content: &sunrise_mod_package::Content,
    known: &BTreeSet<String>,
) -> Result<()> {
    for (id, components) in content.items.iter().map(|i| (&i.id, &i.components)).chain(
        content
            .blocks
            .iter()
            .filter(|b| b.item)
            .map(|b| (&b.id, &b.components)),
    ) {
        validate_components_for(components, known).map_err(|error| format!("{id}: {error}"))?;
    }
    Ok(())
}

fn validate_components_for(
    components: &BTreeMap<String, serde_json::Value>,
    known: &BTreeSet<String>,
) -> Result<()> {
    let patch = component_patch(components)?;
    Ok(patch_components(&mut J::Obj(Vec::new()), &patch, known)?)
}

/// An item's component patch on its entry; `known` holds every type a vanilla item has, so typos fail.
fn patch_components(
    entry: &mut J,
    patch: &[(String, J)],
    known: &BTreeSet<String>,
) -> std::result::Result<(), String> {
    for (key, value) in patch {
        let kind = key.strip_prefix('!').unwrap_or(key);
        if !known.contains(kind) && sunrise_protocol::item::custom::id(kind).is_none() {
            return Err(format!("{kind} is not a known component type"));
        }
        if kind == key {
            entry.set(key, value.clone());
        } else {
            entry.remove(kind);
        }
    }
    Ok(())
}

/// Names a block's item by its own key, `item.<namespace>.<path>` (an `Item.Properties` without
/// `useBlockDescriptionPrefix`), unless the patch already sets or removes its name.
fn own_item_name(id: &str, components: &mut Vec<(String, J)>) {
    const NAME: &str = "minecraft:item_name";
    if components
        .iter()
        .any(|(key, _)| key.strip_prefix('!').unwrap_or(key) == NAME)
    {
        return;
    }
    let key = format!("item.{}", id.replacen(':', ".", 1).replace('/', "."));
    components.push((
        NAME.to_owned(),
        J::Obj(vec![("translate".into(), J::Str(key))]),
    ));
}

/// A declared component patch, as the overlay's JSON.
fn component_patch(components: &BTreeMap<String, serde_json::Value>) -> Result<Vec<(String, J)>> {
    components
        .iter()
        .map(|(key, value)| {
            if !key.starts_with('!') && sunrise_protocol::item::custom::id(key).is_some() {
                let tag = sunrise_game_data::scoreboard::json_to_tag(value);
                sunrise_core::nbt::validate_component_value(&tag)
                    .map_err(|error| format!("component {key}: {error}"))?;
            }
            Ok((key.clone(), serde_json::from_value(value.clone())?))
        })
        .collect()
}

fn read_json(path: &Path) -> Result<J> {
    Ok(serde_json::from_slice(&fs::read(path)?).map_err(|e| format!("{}: {e}", path.display()))?)
}

impl Plan<'_> {
    /// `entity-types.json` with a row for each mod type: its template's, so its
    /// size, eyes and category remain the template's (eyes, spawners and `reg.entity_types`),
    /// with `template`, attachments and attribute bases (`MobTypes`
    /// makes the mob from it). Template values stay raw text: rewriting a float
    /// such as a zombie's `movement_speed` can come back one ulp off.
    fn entity_types_json(&self, bytes: &[u8]) -> std::result::Result<Vec<u8>, String> {
        use serde_json::value::RawValue;
        let rows: BTreeMap<String, Box<RawValue>> =
            serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        let mut out = bytes.trim_ascii_end().to_vec();
        if out.pop() != Some(b'}') {
            return Err("not an object".into());
        }
        let mut empty = rows.is_empty();
        for t in &self.entity_types {
            if rows.contains_key(&t.id) {
                continue;
            }
            let Some(row) = rows.get(&t.template) else {
                continue;
            };
            if !std::mem::take(&mut empty) {
                out.push(b',');
            }
            out.extend(serde_json::to_vec(&t.id).map_err(|e| e.to_string())?);
            out.push(b':');
            out.extend(mod_entity_row(row, t)?);
        }
        out.push(b'}');
        Ok(out)
    }
}
