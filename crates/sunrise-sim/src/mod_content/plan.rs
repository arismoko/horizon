// Included in mod_content.rs to retain its private scope.
impl<'a> Plan<'a> {
    fn new(base: &Path, mods: &'a [ContentMod]) -> Result<Self> {
        let vanilla_items = builtin_item_keys(base)?;
        // Class chains are most-derived first. Fluid-dependent overrides must
        // use the resolved state's fluid, not the template's cached dry answer.
        let mut skylight_rules = BTreeMap::new();
        let mut custom_dampening = BTreeSet::new();
        let needs_waterlogging = mods.iter().any(|m| {
            m.content
                .blocks
                .iter()
                .any(|b| b.state_properties.contains_key("waterlogged"))
        });
        let mut water_states = Vec::new();
        let mut liquid_templates = BTreeSet::new();
        let mut speleothems = BTreeSet::new();
        if base.join("block-behaviours.json").exists() {
            let behaviors = read_json(&base.join("block-behaviours.json"))?;
            if needs_waterlogging {
                let Some(J::Arr(states)) = behaviors.get("states") else {
                    return Err("block-behaviours.json has no state fluids".into());
                };
                for state in states {
                    // Compact state row column 2 is the exported fluid registry ordinal,
                    // also decoded by sunrise_world::level::data::build_with_gameplay.
                    let kind = match state {
                        J::Arr(row) => row.get(2).and_then(J::as_u32),
                        _ => None,
                    }
                    .and_then(|index| FluidKind::ALL.get(index as usize))
                    .ok_or("block-behaviours.json has an invalid state fluid")?;
                    water_states.push(matches!(kind, FluidKind::Water | FluidKind::FlowingWater));
                }
            }
            let Some(J::Arr(blocks)) = behaviors.get("blocks") else {
                return Err("block-behaviours.json has no blocks".into());
            };
            for block in blocks {
                let (Some(J::Str(name)), Some(J::Arr(classes))) =
                    (block.get("name"), block.get("class"))
                else {
                    continue;
                };
                let classes: Vec<&str> = classes
                    .iter()
                    .filter_map(|class| match class {
                        J::Str(name) => Some(name.as_str()),
                        _ => None,
                    })
                    .collect();
                if classes.contains(&"SpeleothemBlock") {
                    speleothems.insert(name.clone());
                }
                if let Some(rule) = classes.iter().find_map(|class| match *class {
                    "BarrierBlock"
                    | "CrossCollisionBlock"
                    | "GlowLichenBlock"
                    | "LightBlock"
                    | "VegetationBlock"
                    | "WallBlock" => Some(SkylightRule::Fluid),
                    "BambooStalkBlock" | "HangingMossBlock" | "LiquidBlock"
                    | "MossyCarpetBlock" | "PipeBlock" | "ShulkerBoxBlock" | "TintedGlassBlock"
                    | "TransparentBlock" | "VineBlock" => Some(SkylightRule::Template),
                    _ => None,
                }) {
                    skylight_rules.insert(name.clone(), rule);
                }
                if classes
                    .iter()
                    .any(|class| matches!(*class, "LeavesBlock" | "TintedGlassBlock"))
                {
                    custom_dampening.insert(name.clone());
                }
                if needs_waterlogging && classes.contains(&"LiquidBlock") {
                    liquid_templates.insert(name.clone());
                }
            }
        }
        let mut force_solid = BTreeMap::new();
        if let Ok(bytes) = fs::read(base.join("worldgen/blocks/block-properties.nbt")) {
            let (named, _) = nbt::read_named(&bytes, Limits::tables())
                .map_err(|e| format!("block-properties.nbt: {e}"))?;
            if let Some(Tag::Compound(blocks)) = named.tag.get("blocks") {
                for (name, flags) in blocks {
                    // Bit 0 is `forceSolidOn`, bit 1 `forceSolidOff`; force-on wins.
                    if let (Ok(name), Tag::Byte(flags)) = (name.to_string(), flags) {
                        match (flags & 1 != 0, flags & 2 != 0) {
                            (true, _) => force_solid.insert(name, true),
                            (_, true) => force_solid.insert(name, false),
                            _ => None,
                        };
                    }
                }
            }
        }
        let facts_path = base.join("facts.json");
        let facts_bytes = fs::read(&facts_path)?;
        let mut facts: J = serde_json::from_slice(&facts_bytes)
            .map_err(|e| format!("{}: {e}", facts_path.display()))?;
        let names: Vec<String> = match facts.at("states") {
            Some(J::Arr(states)) => states
                .iter()
                .map(|s| match s {
                    J::Arr(row) => match row.first() {
                        Some(J::Str(name)) => Ok(name.clone()),
                        _ => Err("facts.json: a state without a name"),
                    },
                    _ => Err("facts.json: a state that is not a row"),
                })
                .collect::<std::result::Result<_, _>>()?,
            _ => return Err("facts.json: no states".into()),
        };
        let defaults: BTreeMap<String, u32> = match facts.at("defaults") {
            Some(J::Obj(o)) => o
                .iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_u32()?)))
                .collect(),
            _ => return Err("facts.json: no defaults".into()),
        };
        let range = |name: &str| -> Option<Range<u32>> {
            let start = names.iter().position(|n| n == name)?;
            let len = names[start..].iter().take_while(|n| *n == name).count();
            // A block's states are one run in registry order.
            (names[start + len..].iter().all(|n| n != name))
                .then_some(start as u32..(start + len) as u32)
        };
        // Native declarations can share a namespace with another package, not a registry key.
        let mut declared_ids = BTreeSet::new();
        let mut item_ids = BTreeSet::new();
        for m in mods {
            for block in &m.content.blocks {
                if defaults.contains_key(&block.id) || !declared_ids.insert(block.id.as_str()) {
                    return Err(format!("{}: {} is already a block", m.id, block.id).into());
                }
            }
            for id in declared_item_ids(m) {
                if vanilla_items.contains(id) || !item_ids.insert(id) {
                    return Err(format!("{}: {id} is already an item", m.id).into());
                }
            }
        }
        let mut block_entity_types: Vec<(&str, &BlockEntityType)> = mods
            .iter()
            .flat_map(|m| {
                m.content
                    .block_entity_types
                    .iter()
                    .map(move |kind| (m.id.as_str(), kind))
            })
            .collect();
        block_entity_types.sort_by(|(_, a), (_, b)| a.id.cmp(&b.id));
        let mut block_entity_factories = BTreeMap::new();
        if !block_entity_types.is_empty() {
            let registry = read_json(&base.join("registries.json"))?;
            let Some(J::Obj(entries)) = registry
                .get("minecraft:block_entity_type")
                .and_then(|r| r.get("entries"))
            else {
                return Err("no minecraft:block_entity_type entries".into());
            };
            let mut ids: BTreeSet<&str> = entries.iter().map(|(id, _)| id.as_str()).collect();
            for (mod_id, kind) in &block_entity_types {
                if !ids.insert(&kind.id) {
                    return Err(format!("block entity type {} is already declared", kind.id).into());
                }
                for block in &kind.blocks {
                    if defaults.contains_key(block.as_str()) {
                        return Err(format!(
                            "mod {mod_id}: block entity type {} cannot own vanilla block {block}",
                            kind.id
                        )
                        .into());
                    }
                    if !declared_ids.contains(block.as_str()) {
                        return Err(format!(
                            "block entity type {}: {block} is not a block",
                            kind.id
                        )
                        .into());
                    }
                    if let Some(previous) =
                        block_entity_factories.insert(block.as_str(), kind.id.as_str())
                    {
                        return Err(format!(
                            "block {block} belongs to both block entity types {previous} and {}",
                            kind.id
                        )
                        .into());
                    }
                }
            }
        }
        let effects = mob_effects(mods);
        let mut stew_effect_ids = BTreeSet::new();
        if mods.iter().flat_map(|m| &m.content.blocks).any(|b| {
            b.suspicious_stew_effects
                .as_ref()
                .is_some_and(|effects| !effects.is_empty())
        }) {
            let registry = read_json(&base.join("registries.json"))?;
            let Some(J::Obj(entries)) = registry
                .get("minecraft:mob_effect")
                .and_then(|r| r.get("entries"))
            else {
                return Err("no minecraft:mob_effect entries".into());
            };
            stew_effect_ids.extend(entries.iter().map(|(id, _)| id.clone()));
            stew_effect_ids.extend(effects.iter().map(|effect| effect.id.clone()));
        }
        let mut blocks = Vec::new();
        let mut next = names.len() as u32;
        let mut sorted: Vec<&ContentMod> = mods.iter().collect();
        sorted.sort_by(|a, b| a.id.cmp(&b.id));
        let mut wanted_sounds = Vec::new();
        let template_registry = std::cell::OnceCell::new();
        let mut template_occlusion = None;
        for m in &sorted {
            let mut declared: Vec<&Block> = m.content.blocks.iter().collect();
            declared.sort_by(|a, b| a.id.cmp(&b.id));
            for decl in declared {
                let stew_effects = decl
                    .suspicious_stew_effects
                    .as_ref()
                    .map(|effects| {
                        effects
                            .iter()
                            .map(|effect| {
                                let id = sunrise_core::identifier::Identifier::parse(&effect.id)
                                    .ok()
                                    .map(|id| id.to_string());
                                let id = id.filter(|id| stew_effect_ids.contains(id)).ok_or_else(
                                    || {
                                        format!(
                                            "{}: suspicious_stew_effects: unknown mob effect {:?}",
                                            decl.id, effect.id
                                        )
                                    },
                                )?;
                                Ok::<_, String>(J::Obj(vec![
                                    ("id".into(), J::Str(id)),
                                    ("duration".into(), J::Num(effect.duration.into())),
                                ]))
                            })
                            .collect::<std::result::Result<Vec<_>, _>>()
                            .map(J::Arr)
                    })
                    .transpose()?;
                let base_block = match decl.base_block.as_deref() {
                    Some(base) if defaults.contains_key(base) || declared_ids.contains(base) => {
                        Some(KnownBlock(base))
                    }
                    Some(base) => {
                        return Err(UnknownBaseBlock {
                            block: decl.id.clone(),
                            base: base.to_owned(),
                        }
                        .into());
                    }
                    None => None,
                };
                if decl.shapes_from_block_entity
                    && !block_entity_factories.contains_key(decl.id.as_str())
                {
                    return Err(format!(
                        "{}: shapes_from_block_entity needs a block entity type that owns the block",
                        decl.id
                    )
                    .into());
                }
                let states = range(&decl.template).ok_or_else(|| {
                    format!("{}: template {} is not a block", decl.id, decl.template)
                })?;
                let pipe_shapes = if let Some(size) = decl.thickness {
                    let Some(J::Arr(rows)) = facts.get("states") else {
                        unreachable!()
                    };
                    let shapes = states.clone().map(|id| {
                        let props = match &rows[id as usize] {
                            J::Arr(row) => row.get(4).and_then(|p| p.get("properties")),
                            _ => None,
                        };
                        let sides = ["down", "up", "north", "south", "west", "east"]
                            .map(|k| matches!(props.and_then(|p| p.get(k)), Some(J::Str(v)) if v == "true"));
                        sunrise_world::collision::pipe_shape(size.into(), sides)
                    });
                    Some(shapes.collect::<std::io::Result<_>>()?)
                } else {
                    None
                };
                let mut default = *defaults.get(&decl.template).ok_or_else(|| {
                    format!(
                        "{}: template {} has no default state",
                        decl.id, decl.template
                    )
                })?;
                let Some(J::Arr(rows)) = facts.get("states") else {
                    unreachable!()
                };
                let is_air = matches!(
                    rows.get(default as usize),
                    Some(J::Arr(row)) if matches!(row.get(1), Some(J::Bool(true)))
                );
                let properties = |id: u32| -> Option<&J> {
                    let J::Arr(row) = rows.get(id as usize)? else {
                        return None;
                    };
                    row.get(4)?.get("properties")
                };
                let declaration_source = || {
                    // File declarations take their id from this path. A code declaration
                    // in the same mod has no file, so name only its block id.
                    if let Some((namespace, path)) = decl.id.split_once(':') {
                        let file = format!("data/{namespace}/sunrise/block/{path}.json");
                        if m.content.files.binary_search(&file).is_ok() {
                            return format!("{file}: {}", decl.id);
                        }
                    }
                    decl.id.clone()
                };
                for key in decl.state_properties.keys() {
                    if properties(default).is_some_and(|p| p.get(key).is_some()) {
                        return Err(format!(
                            "{}: state_properties.{key} is already on {}",
                            declaration_source(),
                            decl.template
                        )
                        .into());
                    }
                }
                if decl.state_properties.contains_key("waterlogged") {
                    if liquid_templates.contains(&decl.template) {
                        return Err(format!(
                            "{}: state_properties.waterlogged cannot be added: template {} is a standalone liquid block",
                            declaration_source(), decl.template
                        ).into());
                    }
                    let template_water = water_states
                        .get(states.start as usize..states.end as usize)
                        .ok_or("block-behaviours.json is missing template state fluids")?;
                    if template_water.iter().all(|water| *water) {
                        return Err(format!(
                            "{}: state_properties.waterlogged cannot be added: every state of template {} already holds water",
                            declaration_source(), decl.template
                        ).into());
                    }
                }
                let combos = combinations(&decl.state_properties, &decl.default_state)
                    .map_err(|e| format!("{}: {e}", decl.id))?;
                let mut known: BTreeMap<String, BTreeSet<String>> = decl
                    .state_properties
                    .iter()
                    .map(|(name, property)| {
                        (
                            name.clone(),
                            property.values().into_iter().map(str::to_owned).collect(),
                        )
                    })
                    .collect();
                for id in states.clone() {
                    if let Some(J::Obj(own)) = properties(id) {
                        for (name, value) in own {
                            if let J::Str(value) = value {
                                known.entry(name.clone()).or_default().insert(value.clone());
                            }
                        }
                    }
                }
                for (key, rules) in [
                    (
                        "skip_rendering",
                        decl.properties.skip_rendering.as_deref().unwrap_or(&[]),
                    ),
                    (
                        "hides_neighbor_face",
                        &decl.properties.hides_neighbor_face[..],
                    ),
                ] {
                    check_face_rule_properties(key, rules, &known)
                        .map_err(|e| format!("{}: {e}", decl.id))?;
                }
                // A `default_state` key the block itself declares is its own,
                // and `combinations` already checks that value against the
                // property. Only a key the template already has narrows which
                // of the template's states is the default, and only that needs
                // the template's own properties: a template with none (`stone`
                // has a single state) can still be given a property and have
                // it defaulted.
                let on_template = decl
                    .default_state
                    .keys()
                    .any(|key| !decl.state_properties.contains_key(key));
                if on_template {
                    let mut wanted = properties(default).cloned().ok_or_else(|| {
                        format!("{}: template default has no state properties", decl.id)
                    })?;
                    for (key, value) in &decl.default_state {
                        if decl.state_properties.contains_key(key) {
                            continue;
                        }
                        if wanted.get(key).is_none() {
                            return Err(format!(
                                "{}: unknown state property {key} on {}",
                                decl.id, decl.template
                            )
                            .into());
                        }
                        wanted.set(key, J::Str(value.clone()));
                    }
                    let matches: Vec<u32> = states
                        .clone()
                        .filter(|id| {
                            properties(*id).is_some_and(|p| {
                                let (J::Obj(a), J::Obj(b)) = (p, &wanted) else {
                                    return false;
                                };
                                a.len() == b.len() && b.iter().all(|(k, v)| p.get(k) == Some(v))
                            })
                        })
                        .collect();
                    if matches.len() != 1 {
                        return Err(format!(
                            "{}: default_state {:?} matches {} states of {}",
                            decl.id,
                            decl.default_state,
                            matches.len(),
                            decl.template
                        )
                        .into());
                    }
                    default = matches[0];
                }
                // The game reads a state table as vanilla's `StateDefinition` does: every
                // property, the template's and the declared ones, sorted by name, the first
                // name slowest. A declared name that sorts before the template's has to
                // interleave with them, so the states are laid out in that order.
                let declared: Vec<(String, u32)> = decl
                    .state_properties
                    .iter()
                    .map(|(name, property)| (name.clone(), property.values().len() as u32))
                    .collect();
                let order = block_order(&states, properties, &declared, combos.len() as u32);
                if decl.context_collision == Some(ContextCollision::Scaffolding) {
                    for &(state, combo) in &order {
                        let value = |name| {
                            state_value(
                                &combos[combo as usize],
                                properties(states.start + state),
                                name,
                            )
                        };
                        ScaffoldingCollision::from_values(value("distance"), value("bottom"))
                            .map_err(|e| {
                                format!("{}: context_collision scaffolding: {e}", decl.id)
                            })?;
                    }
                }
                let resolve = |spec: &BlockShapes| {
                    resolve_shapes(spec, &decl.id, &states, &order, &combos, properties)
                };
                let shapes = if let Some(spec) = &decl.shapes {
                    Some(resolve(spec)?)
                } else {
                    pipe_shapes.map(|shapes: Vec<VoxelShape>| {
                        order
                            .iter()
                            .map(|&(state, _)| shapes[state as usize].clone())
                            .collect()
                    })
                };
                let collision_shapes = decl.collision_shapes.as_ref().map(resolve).transpose()?;
                let mut template_occlusion_full = Vec::new();
                if decl.properties.occludes == Some(true) && shapes.is_none() {
                    use sunrise_world::{
                        collision::CachedCollisionRegistry,
                        registry::{BlockRegistry, BlockStateId, BlockTableFile},
                    };
                    let cached = match &template_occlusion {
                        Some(cached) => cached,
                        None => {
                            let (registry, _) = BlockRegistry::from_facts_json_bytes(&facts_bytes)?;
                            let registry = template_registry.get_or_init(|| registry);
                            let file = BlockTableFile::open(&base.join("occlusion.nbt"), registry)?;
                            template_occlusion.insert(CachedCollisionRegistry::from_table(
                                file.table("occlusion")?,
                            )?)
                        }
                    };
                    for &(state, _) in &order {
                        let shape = cached
                            .cached_shape(BlockStateId(states.start + state))?
                            .ok_or("template has no published occlusion shape")?;
                        template_occlusion_full
                            .push(!shape.join_is_not_empty(&VoxelShape::block(), ShapeOp::XOR)?);
                    }
                }
                let mut has_fluid = Vec::new();
                if shapes.is_some() {
                    for &(state, combo) in &order {
                        let J::Arr(row) = &rows[(states.start + state) as usize] else {
                            unreachable!()
                        };
                        // facts.json exports !BlockState.getFluidState().isEmpty(),
                        // including water in bubble columns and aquatic plants.
                        let Some(J::Bool(fluid)) = row.get(2) else {
                            return Err("facts.json: a state without a fluid flag".into());
                        };
                        has_fluid.push(
                            *fluid
                                || combos[combo as usize]
                                    .iter()
                                    .any(|(name, value)| name == "waterlogged" && value == "true"),
                        );
                    }
                }
                let mut context_shapes = BTreeMap::new();
                for (channel, specs) in [
                    ("outline", &decl.outline_context_shapes),
                    ("collision", &decl.collision_context_shapes),
                ] {
                    for (kind, spec) in specs {
                        context_shapes.insert(format!("{channel}_{}", kind.name()), resolve(spec)?);
                    }
                }
                if let Some(spec) = &decl.interaction_shapes {
                    context_shapes.insert("interaction".into(), resolve(spec)?);
                }
                let state_light = decl
                    .state_light
                    .as_ref()
                    .map(|spec| {
                        resolve_state_values(spec, &decl.id, &states, &order, &combos, properties)
                            .map(|values| values.into_iter().copied().collect())
                    })
                    .transpose()?;
                // The lookup from (template state, combination) to the state's row.
                let mut rows = vec![0; order.len()];
                for (position, &(state, combo)) in order.iter().enumerate() {
                    rows[state as usize * combos.len() + combo as usize] = position as u32;
                }
                let mut rename = Rename::new(&decl.template, &decl.id);
                rename.first = next;
                rename.fanout = combos.len() as u32;
                rename.rows = rows;
                let count =
                    u32::try_from(states.len() as u64 * combos.len() as u64).unwrap_or(u32::MAX);
                next = next.saturating_add(count);
                rename.states = states;
                if let Some(s) = &decl.properties.sounds {
                    if !defaults.contains_key(s) {
                        return Err(format!("{}: sounds {s} is not a block", decl.id).into());
                    }
                    wanted_sounds.push(s.clone());
                }
                blocks.push(NewBlock {
                    stew_effects,
                    decl,
                    base_block,
                    rename,
                    item: None,
                    shapes,
                    collision_shapes,
                    is_air,
                    has_fluid,
                    offset_limits: Vec::new(),
                    template_occlusion_full,
                    skylight: skylight_rules
                        .get(&decl.template)
                        .copied()
                        .unwrap_or(SkylightRule::Shape),
                    custom_dampening: custom_dampening.contains(&decl.template),
                    force_solid: force_solid.get(&decl.template).copied(),
                    template_emission: Vec::new(),
                    context_shapes,
                    state_light,
                    default,
                    combos,
                    order,
                    wall_of: None,
                });
            }
        }
        // A sign's wall form, which `Content` checked against its mod's blocks.
        let pairs: Vec<_> = blocks
            .iter()
            .filter_map(|b| Some((b.decl.wall_block.clone()?, b.decl.id.clone())))
            .collect();
        for (wall, sign) in pairs {
            if let Some(block) = blocks.iter_mut().find(|b| b.decl.id == wall) {
                block.wall_of = Some(sign);
            }
        }
        if next > MAX_STATES {
            let mut by_mod: BTreeMap<&str, u32> = BTreeMap::new();
            for b in &blocks {
                *by_mod
                    .entry(&b.decl.id[..b.decl.id.find(':').unwrap_or(0)])
                    .or_default() += (b.rename.states.len() as u32).saturating_mul(b.rename.fanout);
            }
            let list: Vec<String> = by_mod.iter().map(|(m, n)| format!("{m} {n}")).collect();
            return Err(format!(
                "{next} block states, more than the {MAX_STATES} a world can hold ({} in the game; \
                 the mods: {})",
                names.len(),
                list.join(", ")
            )
            .into());
        }
        // Block items copy the template block's item.
        let gameplay = read_json(&base.join("gameplay-facts.json"))?;
        let rows = match gameplay.get("states") {
            Some(J::Arr(rows)) if rows.len() == names.len() => rows,
            _ => return Err("gameplay-facts.json does not match facts.json".into()),
        };
        let mut sounds = BTreeMap::new();
        for s in wanted_sounds {
            let row = &rows[defaults[&s] as usize];
            sounds.insert(s, row.get("sound").cloned().unwrap_or(J::Null));
        }
        let mut items = Vec::new();
        for b in &mut blocks {
            let p = &b.decl.properties;
            let inherited = crate::blocks::inherited_offset_limits(
                &b.decl.template,
                speleothems.contains(&b.decl.template),
            );
            b.offset_limits = b
                .order
                .iter()
                .map(|(state, _)| {
                    use sunrise_mod_package::content::OffsetType;
                    let enabled = match p.offset {
                        Some(OffsetType::None) => false,
                        Some(_) => true,
                        None => matches!(
                            rows[(b.rename.states.start + state) as usize].get("offset"),
                            Some(J::Bool(true))
                        ),
                    };
                    enabled.then(|| {
                        let defaults = match p.offset {
                            Some(OffsetType::Xz) => (0.25, 0.0),
                            Some(OffsetType::Xyz) => (0.25, 0.2),
                            _ => inherited,
                        };
                        (
                            p.max_horizontal_offset.unwrap_or(defaults.0),
                            if p.offset == Some(OffsetType::Xz)
                                || (p.offset.is_none() && inherited.1 == 0.0)
                            {
                                0.0
                            } else {
                                p.max_vertical_offset.unwrap_or(defaults.1)
                            },
                        )
                    })
                })
                .collect();
            if p.spawn_rule == Some(SpawnRule::Default) {
                b.template_emission = b
                    .order
                    .iter()
                    .map(|(state, _)| {
                        rows[(b.rename.states.start + state) as usize]
                            .get("emission")
                            .and_then(J::as_u32)
                            .map_or(0, |light| light as u8)
                    })
                    .collect();
            }
            let row = &rows[b.rename.states.start as usize];
            b.item = match row.get("item") {
                Some(J::Str(i)) if i != "minecraft:air" => Some(i.clone()),
                _ => None,
            };
            if b.decl.item {
                let item = b.item.as_deref().ok_or_else(|| {
                    format!("{}: template {} has no item", b.decl.id, b.decl.template)
                })?;
                let mut components = component_patch(&b.decl.components)
                    .map_err(|error| format!("{}: {error}", b.decl.id))?;
                if b.decl.own_item_name {
                    own_item_name(&b.decl.id, &mut components);
                }
                items.push(NewItem {
                    rename: Rename::new(item, &b.decl.id),
                    max_stack_size: None,
                    rarity: None,
                    components,
                    crafting_remainder: None,
                    block: None,
                    arm_pose: None,
                    wall_block: b.decl.wall_block.clone(),
                    template: None,
                });
            }
        }
        for m in &sorted {
            for i in &m.content.items {
                if let Some(block) = &i.block
                    && !defaults.contains_key(block)
                    && !blocks.iter().any(|b| b.decl.id == *block)
                {
                    return Err(format!("{}: block {block:?} is not a block", i.id).into());
                }
                items.push(NewItem {
                    rename: Rename::new(&i.template, &i.id),
                    max_stack_size: i.max_stack_size,
                    rarity: i.rarity,
                    components: component_patch(&i.components)
                        .map_err(|error| format!("{}: {error}", i.id))?,
                    crafting_remainder: i.crafting_remainder.clone(),
                    block: i.block.clone(),
                    arm_pose: i.arm_pose,
                    wall_block: None,
                    template: i.block.is_none().then(|| i.template.clone()),
                });
            }
        }
        // Block items and standalone items share the same registry. Sort
        // them together by mod ID and entry name before assigning IDs.
        items.sort_by(|a, b| a.rename.to.cmp(&b.rename.to));
        let mut tabs = Vec::new();
        let mut lang = BTreeMap::new();
        for m in &sorted {
            tabs.extend(m.content.creative_tabs.iter());
            for ns in &m.content.namespaces {
                let Ok(bytes) = m.content.file(&format!("assets/{ns}/lang/en_us.json")) else {
                    continue;
                };
                if let Ok(map) = serde_json::from_slice::<BTreeMap<String, String>>(&bytes) {
                    lang.extend(map);
                }
            }
        }
        Ok(Plan {
            blocks,
            items,
            vanilla_items,
            effects,
            fluids: fluids(mods),
            entity_types: entity_types(mods),
            block_entity_types: block_entity_types
                .into_iter()
                .map(|(_, kind)| kind)
                .collect(),
            block_entity_factories,
            menu_types: menu_types(mods).into_iter().map(|(menu, _)| menu).collect(),
            states: names.len(),
            defaults,
            sounds,
            tabs,
            names: lang,
        })
    }

    /// What moves in a new block's copies of its template's rows: the
    /// block, and the template's item when it has another name. A block
    /// without an item gets `minecraft:air` where rows name one, except a
    /// sign's wall form, whose item is the paired sign's.
    fn renames(&self, b: &NewBlock) -> Vec<Rename> {
        let mut out = vec![b.rename.clone()];
        let own = b.decl.item.then_some(b.decl.id.as_str());
        if let Some(item) = b.item.as_deref().filter(|i| *i != b.decl.template)
            && let Some(to) = own.or(b.wall_of.as_deref())
        {
            out.push(Rename::new(item, to));
        }
        out
    }

    /// Appends each new block's copies of the template's states to `list`,
    /// writing the declared overrides in the table's row format.
    fn states(&self, list: &mut Vec<J>, ids: bool, name: &str) -> Result<()> {
        if list.len() != self.states {
            return Err(format!("{} states, not {}", list.len(), self.states).into());
        }
        for b in &self.blocks {
            let renames = self.renames(b);
            // One row per template state per combination the block declares,
            // so its states stay one run in registry order.
            for (next, (id, combo)) in (b.rename.first..).zip(b.state_combinations()) {
                let state_index = (next - b.rename.first) as usize;
                let mut row = list[id as usize].clone();
                for r in &renames {
                    row.rename(r, ids);
                }
                if !combo.is_empty() {
                    set_state_properties(&mut row, combo);
                }
                if matches!(
                    name,
                    "block-behaviours.json"
                        | "facts.json"
                        | "mob-data.json"
                        | "worldgen/block_flags.json"
                ) && combo
                    .iter()
                    .any(|(key, value)| key == "waterlogged" && value == "true")
                {
                    let J::Arr(columns) = &mut row else {
                        return Err(format!("{name}: missing state row {id}").into());
                    };
                    if name == "block-behaviours.json" {
                        let fluid = columns
                            .get_mut(2..5)
                            .ok_or("block-behaviours.json: missing state fluid")?;
                        let water = FluidState::WATER;
                        let ordinal = FluidKind::ALL
                            .iter()
                            .position(|&kind| kind == water.kind())
                            .expect("source water belongs to the vanilla fluid registry");
                        fluid.clone_from_slice(&[
                            J::Num(ordinal.into()),
                            J::Num(water.amount().into()),
                            J::Bool(water.falling()),
                        ]);
                    } else if name == "mob-data.json" {
                        let [path, flags, ..] = columns.as_mut_slice() else {
                            return Err("mob-data.json: missing state pathfinding data".into());
                        };
                        // Only the walk evaluator's generic OPEN result looks at the fluid here.
                        // Air, blocked and special paths keep their template's answer.
                        if !b.is_air && path.as_str() == Some("OPEN") {
                            *path = J::Str("WATER".into());
                        }
                        let mask = flags.as_u32().ok_or("mob-data.json: invalid state flags")?;
                        // Inherited waterlogged properties never take this branch: their exported
                        // WATER answer is kept. For a new declaration use BlockBehaviour's default.
                        let water =
                            u32::from(PartAnswers::pathfindable_bit(PathComputationType::Water));
                        *flags = J::Num(((mask | water) & !u32::from(VALID_EMPTY_SPAWN)).into());
                    } else {
                        let has_fluid = if name == "facts.json" { 2 } else { 1 };
                        *columns
                            .get_mut(has_fluid)
                            .ok_or_else(|| format!("{name}: missing state fluid flag"))? =
                            J::Bool(true);
                    }
                }
                if name == "mob-data.json"
                    && let Some(rule) = b.decl.properties.spawn_rule
                {
                    let J::Arr(columns) = &mut row else {
                        return Err(format!("{name}: missing state row {id}").into());
                    };
                    let flags = columns
                        .get_mut(1)
                        .ok_or("mob-data.json: missing state flags")?;
                    let mask = flags.as_u32().ok_or("mob-data.json: invalid state flags")?;
                    let spawn = u32::from(VALID_SPAWN | VALID_SPAWN_LEAVES | VALID_SPAWN_ICE);
                    let allowed = match rule {
                        SpawnRule::Always => spawn,
                        SpawnRule::Never => 0,
                        SpawnRule::OcelotOrParrot => u32::from(VALID_SPAWN_LEAVES),
                        SpawnRule::Default => {
                            let light = match (&b.state_light, b.decl.properties.light) {
                                (Some(values), _) => values[state_index],
                                (None, Some(light)) => light,
                                (None, None) => b.template_emission[state_index],
                            };
                            if mask & u32::from(STURDY_TOP) != 0 && light < 14 {
                                spawn
                            } else {
                                0
                            }
                        }
                    };
                    *flags = J::Num(((mask & !spawn) | allowed).into());
                }
                if name == "gameplay-facts.json" {
                    row.set("id", J::Num(next.into()));
                    if let Some(item) = b.item_row() {
                        row.set("item", J::Str(item.into()));
                    }
                    if let Some(item) = &b.decl.pick_item {
                        row.set("pick_item", J::Str(item.clone()));
                    }
                    self.override_gameplay(b, state_index, combo, &mut row);
                    if let Some(shapes) = &b.shapes {
                        let shape = &shapes[state_index];
                        let boxes = shape
                            .boxes()
                            .iter()
                            .map(|b| {
                                J::Arr(
                                    [
                                        b.min(Axis::X),
                                        b.min(Axis::Y),
                                        b.min(Axis::Z),
                                        b.max(Axis::X),
                                        b.max(Axis::Y),
                                        b.max(Axis::Z),
                                    ]
                                    .into_iter()
                                    .map(J::f64)
                                    .collect(),
                                )
                            })
                            .collect();
                        // BlockState.initCache and BlockBehaviour.getLightDampening:
                        // the occlusion shape uses the offset at the cache's origin.
                        let occlusion = b.declared_occlusion(state_index, shape);
                        let full =
                            !occlusion.join_is_not_empty(&VoxelShape::block(), ShapeOp::XOR)?;
                        let can_occlude = matches!(row.get("can_occlude"), Some(J::Bool(true)));
                        let solid = can_occlude && full;
                        let down = match b.skylight {
                            SkylightRule::Shape => !full && !b.has_fluid[state_index],
                            SkylightRule::Fluid => !b.has_fluid[state_index],
                            SkylightRule::Template => {
                                matches!(row.get("skylight_down"), Some(J::Bool(true)))
                            }
                        };
                        // A partial outline needs directional light occlusion even
                        // when its full-cube template bypasses shape checks. Keep
                        // the template's participation for special full shapes.
                        let shape_occlusion = can_occlude
                            && (!full || matches!(row.get("shape_occlusion"), Some(J::Bool(true))));
                        row.set("shape_occlusion", J::Bool(shape_occlusion));
                        row.set("solid_render", J::Bool(solid));
                        row.set("skylight_down", J::Bool(down));
                        if !b.custom_dampening {
                            row.set(
                                "dampening",
                                J::Num(
                                    (if solid {
                                        15
                                    } else if down {
                                        0
                                    } else {
                                        1
                                    })
                                    .into(),
                                ),
                            );
                        }
                        row.set("outline", J::Arr(boxes));
                        row.set("outline_edges", J::Arr(shape_edges(shape)));
                    }
                    if let Some(shapes) = b.collision_shapes.as_ref().or(b.shapes.as_ref()) {
                        let shape = &shapes[state_index];
                        row.set(
                            "full_collision",
                            J::Bool(!shape.join_is_not_empty(&VoxelShape::block(), ShapeOp::XOR)?),
                        );
                    }
                }
                list.push(row);
            }
        }
        Ok(())
    }

    /// The declared properties, on a gameplay row.
    fn override_gameplay(
        &self,
        b: &NewBlock,
        state_index: usize,
        combo: &[(String, String)],
        row: &mut J,
    ) {
        let p = &b.decl.properties;
        let offset = b.offset_limits[state_index];
        row.set("offset", J::Bool(offset.is_some()));
        if let Some((horizontal, vertical)) = offset {
            row.set(
                "offset_limits",
                J::Arr(vec![J::f64(horizontal as f64), J::f64(vertical as f64)]),
            );
        }
        if b.shapes.is_some() || p.offset == Some(sunrise_mod_package::content::OffsetType::None) {
            row.set("outline_offset", J::Bool(offset.is_some()));
        }
        if let Some(value) = p.ignited_by_lava {
            row.set("ignited_by_lava", J::Bool(value));
        }
        if let Some(value) = p.replaceable {
            row.set("replaceable", J::Bool(value));
        }
        if let Some(ticks) = &p.random_ticks {
            row.set("random_ticks", J::Bool(ticks.applies(combo)));
        }
        if p.destroy_overlay == Some(false) {
            row.set("no_destroy_overlay", J::Bool(true));
        }
        if let Some(shape) = p.render_shape {
            row.set("render_shape", J::Str(shape.name().into()));
        }
        if let Some(reaction) = p.push_reaction {
            row.set("push", J::Str(reaction.name().into()));
        }
        // `noOcclusion`: a block that can't occlude is never a solid render either, and a solid
        // render's full light dampening becomes a non-solid block's (`getLightDampening`: 0 when
        // skylight passes down, else 1, as for leaves and spawners).
        if let Some(can_occlude) = p.occludes {
            row.set("can_occlude", J::Bool(can_occlude));
        }
        if p.occludes == Some(true) && b.shapes.is_none() {
            let solid = b.template_occlusion_full[state_index];
            row.set("solid_render", J::Bool(solid));
            if !b.custom_dampening {
                let down = matches!(row.get("skylight_down"), Some(J::Bool(true)));
                row.set(
                    "dampening",
                    J::Num((if solid { 15 } else { i64::from(!down) }).into()),
                );
            }
        }
        if p.occludes == Some(false) {
            let solid = matches!(row.get("solid_render"), Some(J::Bool(true)));
            if solid {
                let down = matches!(row.get("skylight_down"), Some(J::Bool(true)));
                row.set("dampening", J::Num(i64::from(!down).into()));
            }
            row.set("can_occlude", J::Bool(false));
            row.set("solid_render", J::Bool(false));
        }
        if let Some(v) = p.hardness {
            row.set("hardness", J::f64(v as f64));
        }
        if let Some(v) = p.explosion_resistance {
            row.set("explosion_resistance", J::f64(v as f64));
        }
        light_rows::override_light(b, state_index, row);
        if let Some(v) = p.requires_tool {
            row.set("requires_tool", J::Bool(v));
        }
        if let Some(v) = p.friction {
            row.set("friction", J::f64(v as f64));
        }
        if let Some(v) = p.map_color {
            let col = if v == 0 { 0 } else { MAP_COLORS[v as usize] };
            row.set("map_color", J::Num(col.into()));
        }
        if let Some(s) = &p.sounds {
            row.set("sound", self.sounds[s].clone());
        }
    }

    /// Adds each new block's copy of the template's entry to a map by name.
    fn names(&self, map: &mut J, ids: bool) {
        for b in &self.blocks {
            if let Some(v) = map.get(&b.decl.template) {
                let mut v = v.clone();
                for r in &self.renames(b) {
                    v.rename(r, ids);
                }
                map.set(&b.decl.id, v);
            }
        }
    }

    /// Adds each new item's copy of its template's entry to a map by name;
    /// `components` applies the overrides (items.json).
    fn items(&self, map: &mut J, components: bool) -> Result<()> {
        let known = if components {
            component_types(map)
        } else {
            BTreeSet::new()
        };
        for i in &self.items {
            if !components && let Some(block) = &i.block {
                map.set(
                    &i.rename.to,
                    J::Obj(vec![
                        (
                            "class".into(),
                            J::Arr(vec![J::Str("BlockItem".into()), J::Str("Item".into())]),
                        ),
                        (
                            "fields".into(),
                            J::Obj(vec![("block".into(), J::Str(block.clone()))]),
                        ),
                    ]),
                );
                continue;
            }
            let Some(v) = map.get(&i.rename.from) else {
                continue;
            };
            let mut v = v.clone();
            v.rename(&i.rename, false);
            if !components
                && i.template.is_none()
                && let Some(fields) = v.get_mut("fields")
            {
                // The generated BlockItem points to its new block, not to the
                // template's main block if the template shared another item.
                fields.set("block", J::Str(i.rename.to.clone()));
            }
            if let Some(wall) = &i.wall_block
                && let Some(fields) = v.get_mut("fields")
            {
                fields.set("wallBlock", J::Str(wall.clone()));
            }
            if components {
                if let Some(n) = i.max_stack_size {
                    v.set("minecraft:max_stack_size", J::Num(n.into()));
                }
                if let Some(r) = i.rarity {
                    let r = match r {
                        Rarity::Common => "common",
                        Rarity::Uncommon => "uncommon",
                        Rarity::Rare => "rare",
                        Rarity::Epic => "epic",
                    };
                    v.set("minecraft:rarity", J::Str(r.into()));
                }
                patch_components(&mut v, &i.components, &known)
                    .map_err(|e| format!("{}: {e}", i.rename.to))?;
            }
            map.set(&i.rename.to, v);
        }
        if components {
            for b in &self.blocks {
                for (key, item) in [
                    ("block_item", &b.decl.block_item),
                    ("pick_item", &b.decl.pick_item),
                ] {
                    if let Some(item) = item
                        && map.get(item).is_none()
                    {
                        return Err(format!("{}: {key} {item:?} is not an item", b.decl.id).into());
                    }
                }
            }
            for i in &self.items {
                if let Some(rest) = &i.crafting_remainder
                    && (rest == "minecraft:air" || map.get(rest).is_none())
                {
                    return Err(format!(
                        "{}: crafting_remainder {rest:?} is not an item",
                        i.rename.to
                    )
                    .into());
                }
            }
        }
        Ok(())
    }

    /// `BlockStateBase.calculateSolid` of every state that declares a collision shape, over the
    /// template's flag: the legacy solid bit (bit 0 of a state row's flags) follows the declared
    /// shape unless the template forces it.
    fn declared_solid(&self, doc: &mut J) {
        let Some(J::Arr(rows)) = doc.at("states") else {
            return;
        };
        for b in &self.blocks {
            let Some(shapes) = b.collision_shapes.as_ref().or(b.shapes.as_ref()) else {
                continue;
            };
            for (offset, shape) in shapes.iter().enumerate() {
                let Some(J::Arr(row)) = rows.get_mut(b.rename.first as usize + offset) else {
                    continue;
                };
                if let Some(J::Num(flags)) = row.get_mut(1) {
                    *flags =
                        with_solid_bit(flags.as_i64().unwrap_or(0), b.force_solid, shape).into();
                }
            }
        }
    }

    /// One JSON table, rewritten.
    fn json(&self, name: &str, doc: &mut J) -> Result<()> {
        let states = |doc: &mut J, path: &str, ids: bool| -> Result<()> {
            match doc.at(path) {
                Some(J::Arr(a)) => self.states(a, ids, name),
                _ => Err(format!("no list at /{path}").into()),
            }
        };
        match name {
            "facts.json" => {
                states(doc, "states", false)?;
                let defaults = doc.at("defaults").ok_or("no defaults")?;
                for b in &self.blocks {
                    let default = b.default;
                    let id = b.rename.id(default).ok_or("a default outside its block")?;
                    defaults.set(&b.decl.id, J::Num(id.into()));
                }
            }
            "gameplay-facts.json" => states(doc, "states", false)?,
            "creative-inventory.json" => self.creative_tabs(doc)?,
            "block-behaviours.json" => {
                states(doc, "states", false)?;
                if let Some(J::Arr(rows)) = doc.at("states") {
                    for b in &self.blocks {
                        if let Some(instrument) = b.decl.properties.instrument {
                            for (id, _) in (b.rename.first..).zip(b.state_combinations()) {
                                let J::Arr(row) = &mut rows[id as usize] else {
                                    return Err(
                                        "block-behaviours.json: expected a compact state row"
                                            .into(),
                                    );
                                };
                                row.resize(row.len().max(9), J::Null);
                                // Instrument column, read by sunrise_world::level::data's state facts decoder.
                                row[8] = J::Str(instrument.name().to_ascii_uppercase());
                            }
                        }
                    }
                }
                self.declared_solid(doc);
                let Some(J::Arr(blocks)) = doc.at("blocks") else {
                    return Err("no blocks".into());
                };
                for b in &self.blocks {
                    let template = blocks.iter().find(
                        |x| matches!(x.get("name"), Some(J::Str(n)) if *n == b.decl.template),
                    );
                    if let Some(mut v) = template.cloned() {
                        for r in &self.renames(b) {
                            v.rename(r, false);
                        }
                        if let Some(item) = b.item_row() {
                            v.set("item", J::Str(item.into()));
                        }
                        if b.decl.state_properties.contains_key("waterlogged") {
                            if let Some(J::Arr(interfaces)) = v.at("interfaces") {
                                interfaces.push(J::Str("SimpleWaterloggedBlock".into()));
                            }
                            if let Some(fields) = v.at("fields") {
                                fields.set("declaredWaterlogged", J::Bool(true));
                            }
                        }
                        // `SignBlock.getWoodType`: the wood's name is what
                        // the editor reads of it.
                        if let Some(wood) = &b.decl.wood_type
                            && let Some(name) = v.at("fields/type/name")
                        {
                            *name = J::Str(wood.clone());
                        }
                        // Resolve each gate site against the untouched template wood sound.
                        if let Some(fields) = v.at("fields") {
                            for (key, sound) in [
                                ("gateOpenSounds", &b.decl.open_sound),
                                ("gateCloseSounds", &b.decl.close_sound),
                            ] {
                                let Some(sound) = sound else { continue };
                                use sunrise_mod_package::content::GateSound;
                                let sites = match sound {
                                    GateSound::All(sound) => {
                                        [Some(sound), Some(sound), Some(sound)]
                                    }
                                    GateSound::Sites {
                                        on_use,
                                        on_explosion,
                                        on_redstone,
                                    } => [
                                        on_use.as_ref(),
                                        on_explosion.as_ref(),
                                        on_redstone.as_ref(),
                                    ],
                                };
                                fields.set(
                                    key,
                                    J::Arr(
                                        sites
                                            .into_iter()
                                            .map(|sound| {
                                                sound.map_or(J::Null, |sound| J::Str(sound.clone()))
                                            })
                                            .collect(),
                                    ),
                                );
                            }
                        }
                        // `createDefaultSignText`: the colour and glow a new sign's text starts with.
                        if (b.decl.sign_text_color.is_some() || b.decl.sign_text_glow.is_some())
                            && let Some(fields) = v.at("fields")
                        {
                            let color = b.decl.sign_text_color.as_deref().unwrap_or("black");
                            let id = sunrise_mod_package::content::DYE_COLORS
                                .iter()
                                .position(|name| *name == color)
                                .unwrap_or_default();
                            fields.set("sign_text_color", J::Num((id as u64).into()));
                            fields.set(
                                "sign_text_glow",
                                J::Bool(b.decl.sign_text_glow.unwrap_or(false)),
                            );
                        }
                        if let Some(base) = &b.base_block {
                            let field = v
                                .at("fields/baseBlock")
                                .ok_or("farmland template has no base block")?;
                            *field = J::Str(base.0.to_owned());
                        }
                        // A connecting block: the chorus plant's PipeBlock
                        // without its rules, joining the declared tag.
                        if let Some(tag) = &b.decl.connects_to {
                            let classes = ["PipeBlock", "Block"].map(|c| J::Str(c.into()));
                            v.set("class", J::Arr(classes.into()));
                            let mut fields = v
                                .get("fields")
                                .cloned()
                                .unwrap_or(J::Obj(Default::default()));
                            fields.set("connectsTo", J::Str(tag.clone()));
                            v.set("fields", fields);
                        }
                        if b.decl.properties.vegetation {
                            v.set("declared_vegetation", J::Bool(true));
                        }
                        if let Some(effects) = &b.stew_effects {
                            v.set("suspicious_stew_effects", effects.clone());
                        }
                        blocks.push(v);
                    }
                }
                self.items(doc.at("items").ok_or("no items")?, false)?;
                for odds in ["fire/burnOdds", "fire/igniteOdds"] {
                    if let Some(map) = doc.at(odds) {
                        self.names(map, false);
                    }
                }
            }
            "model-states.json" | "render-definitions.json" => {
                let model = name == "model-states.json";
                self.names(doc, !model);
                for b in &self.blocks {
                    let Some(entry) = doc.get_mut(&b.decl.id) else {
                        continue;
                    };
                    if !model {
                        if let Some(rules) = &b.decl.properties.skip_rendering {
                            entry.set("skip_rendering", face_rules_json(rules));
                        }
                        if !b.decl.properties.hides_neighbor_face.is_empty() {
                            entry.set(
                                "hides_neighbor_face",
                                face_rules_json(&b.decl.properties.hides_neighbor_face),
                            );
                        }
                        if let Some(supports) = b.decl.properties.supports_external_face_hiding {
                            entry.set("supports_external_face_hiding", J::Bool(supports));
                        }
                    }
                    // Faces beside a block that occludes nothing are drawn (`Block.shouldRenderFace`).
                    if !model && let Some(can_occlude) = b.decl.properties.occludes {
                        entry.set("can_occlude", J::Bool(can_occlude));
                    }
                    if model && let Some(schema) = entry.get_mut("schema") {
                        for (property_name, property) in &b.decl.state_properties {
                            schema.set(
                                property_name,
                                J::Obj(vec![(
                                    "named".into(),
                                    J::Arr(
                                        property
                                            .values()
                                            .into_iter()
                                            .map(|v| J::Str(v.into()))
                                            .collect(),
                                    ),
                                )]),
                            );
                        }
                    }
                    if let Some(J::Arr(states)) = entry.get_mut("states") {
                        let template = states.clone();
                        let mut expanded = Vec::new();
                        for (index, (id, combo)) in b.state_combinations().enumerate() {
                            if model {
                                let mut state = template
                                    .get((id - b.rename.states.start) as usize)
                                    .ok_or("model states do not match the template")?
                                    .clone();
                                for (name, value) in combo {
                                    state.set(name, J::Str(value.clone()));
                                }
                                expanded.push(state);
                            } else {
                                expanded.push(J::Num((b.rename.first + index as u32).into()));
                            }
                        }
                        *states = expanded;
                    }
                }
            }
            "landing-definitions.json" => {
                self.names(doc, false);
                for b in &self.blocks {
                    if let Some(base) = &b.base_block {
                        let entry = doc
                            .get_mut(&b.decl.id)
                            .ok_or("farmland template has no landing definition")?;
                        entry.set("base", J::Str(base.0.to_owned()));
                    }
                }
            }
            "worldgen/blocks/survival-rules.json" => self.names(doc, false),
            "worldgen/blocks/survival-extra-definitions.json" => {
                if let Some(map) = doc.at("ignition_odds") {
                    self.names(map, false);
                }
            }
            "items.json" => self.items(doc, true)?,
            "registries.json" => {
                // Network ids: the game and its server map items, blocks, mob
                // effects and entity types through this report, so new ones get
                // the ids after vanilla's, in plan order. Chunk packets carry
                // the declared block entity type, not its block's id.
                let new_items = self.items.iter().map(|i| i.rename.to.clone());
                let new_blocks = self.blocks.iter().map(|b| b.decl.id.clone());
                let mut registries = vec![
                    ("minecraft:item", new_items.collect::<Vec<_>>()),
                    ("minecraft:block", new_blocks.collect()),
                ];
                if !self.block_entity_types.is_empty() {
                    let new_types = self.block_entity_types.iter().map(|t| t.id.clone());
                    registries.push(("minecraft:block_entity_type", new_types.collect()));
                }
                if !self.effects.is_empty() {
                    let new_effects = self.effects.iter().map(|e| e.id.clone());
                    registries.push(("minecraft:mob_effect", new_effects.collect()));
                }
                // Fluid tags go by these ids: a mod fluid's source, then its flowing form.
                if !self.fluids.is_empty() {
                    let new_fluids = self
                        .fluids
                        .iter()
                        .flat_map(|f| [f.id.clone(), f.flowing_id()]);
                    registries.push(("minecraft:fluid", new_fluids.collect()));
                }
                if !self.entity_types.is_empty() {
                    let new_types = self.entity_types.iter().map(|t| t.id.clone());
                    registries.push(("minecraft:entity_type", new_types.collect()));
                }
                // `ClientboundOpenScreenPacket` carries a menu type's id.
                if !self.menu_types.is_empty() {
                    let new_menus = self.menu_types.iter().map(|m| m.id.clone());
                    registries.push(("minecraft:menu", new_menus.collect()));
                }
                for (registry, names) in registries {
                    let Some(J::Obj(entries)) =
                        doc.get_mut(registry).and_then(|r| r.get_mut("entries"))
                    else {
                        return Err(format!("no {registry} entries").into());
                    };
                    let next = entries
                        .iter()
                        .filter_map(|(_, e)| match e {
                            J::Obj(o) => o.iter().find(|(k, _)| k == "protocol_id")?.1.as_u32(),
                            _ => None,
                        })
                        .max()
                        .map_or(0, |m| m + 1);
                    for (i, name) in names.into_iter().enumerate() {
                        if entries.iter().any(|(k, _)| *k == name) {
                            continue;
                        }
                        let id = J::Num((next + i as u32).into());
                        entries.push((name, J::Obj(vec![("protocol_id".into(), id)])));
                    }
                }
            }
            "command-registries.json" => {
                self.names(doc.at("blocks").ok_or("no blocks")?, false);
                let new_items = || self.items.iter().map(|i| J::Str(i.rename.to.clone()));
                let new_blocks = || self.blocks.iter().map(|b| J::Str(b.decl.id.clone()));
                let Some(J::Arr(items)) = doc.at("items") else {
                    return Err("no items".into());
                };
                items.extend(new_items());
                // The registries commands check ids against.
                if let Some(J::Arr(ids)) = doc.at("by_registry/minecraft:item/ids") {
                    ids.extend(new_items());
                }
                if let Some(J::Arr(ids)) = doc.at("by_registry/minecraft:block/ids") {
                    ids.extend(new_blocks());
                }
                // `/effect` and the mob_effect resource arguments.
                let new_effects = || self.effects.iter().map(|e| J::Str(e.id.clone()));
                if let Some(J::Arr(ids)) = doc.at("effects") {
                    ids.extend(new_effects());
                }
                if let Some(J::Arr(ids)) = doc.at("by_registry/minecraft:mob_effect/ids") {
                    ids.extend(new_effects());
                }
                // Fluid tag files and fluid arguments.
                if let Some(J::Arr(ids)) = doc.at("by_registry/minecraft:fluid/ids") {
                    ids.extend(
                        self.fluids
                            .iter()
                            .flat_map(|f| [J::Str(f.id.clone()), J::Str(f.flowing_id())]),
                    );
                }
                // entity type arguments and `type=` in selectors.
                let new_types = || self.entity_types.iter().map(|t| J::Str(t.id.clone()));
                if let Some(J::Arr(ids)) = doc.at("entity_types") {
                    ids.extend(new_types());
                }
                if let Some(J::Arr(ids)) = doc.at("by_registry/minecraft:entity_type/ids") {
                    ids.extend(new_types());
                }
            }
            "mob-data.json" => states(doc, "states", false)?,
            "worldgen/block_flags.json" => states(doc, "", false)?,
            "worldgen/blocks/structure-blocks.json" => {
                for key in ["rotate", "mirror", "full_block"] {
                    states(doc, key, true)?;
                }
                let added: usize = self
                    .blocks
                    .iter()
                    .map(|b| b.rename.states.len() * b.combos.len())
                    .sum();
                doc.set("states", J::Num(((self.states + added) as u64).into()));
            }
            _ => return Err("no rule for this table".into()),
        }
        Ok(())
    }

    /// The states whose block entity publishes their shapes (`shapes_from_block_entity`), in
    /// the collision and support tables, which mark such states in their scan metadata.
    fn published_states(&self, tag: &mut Tag) -> Result<()> {
        self.list_states(tag, "published_shapes", |decl| {
            decl.shapes_from_block_entity
        })
    }

    /// The states whose block takes scaffolding's context collision (`context_collision`), in
    /// the collision tables.
    fn scaffolding_states(&self, tag: &mut Tag) -> Result<()> {
        self.list_states(tag, "scaffolding_collision", |decl| {
            decl.context_collision == Some(ContextCollision::Scaffolding)
        })
    }

    /// Lists under `key` the states of the blocks `wanted` picks; no list when it picks none.
    fn list_states(&self, tag: &mut Tag, key: &str, wanted: impl Fn(&Block) -> bool) -> Result<()> {
        let states: Vec<i32> = self
            .blocks
            .iter()
            .filter(|block| wanted(block.decl))
            .flat_map(|block| {
                (0..block.order.len())
                    .map(move |offset| (block.rename.first as usize + offset) as i32)
            })
            .collect();
        if states.is_empty() {
            return Ok(());
        }
        let Tag::Compound(root) = tag else {
            return Err("collision table is not a compound".into());
        };
        root.insert(key.into(), Tag::IntArray(states));
        Ok(())
    }

    fn declared_shapes(&self, tag: &mut Tag, collision: bool) -> Result<()> {
        let Tag::Compound(root) = tag else {
            return Err("collision table is not a compound".into());
        };
        let mut indices = Vec::new();
        let Some(Tag::List(entries)) = root.get_mut(&JavaString::from("shapes")) else {
            return Err("collision table has no shapes".into());
        };
        for block in &self.blocks {
            let shapes = if collision {
                block.collision_shapes.as_ref().or(block.shapes.as_ref())
            } else {
                block.shapes.as_ref()
            };
            if let Some(shapes) = shapes {
                let mut encoded = BTreeMap::new();
                for (offset, shape) in shapes.iter().enumerate() {
                    let occlusion;
                    let shape = if collision {
                        shape
                    } else {
                        occlusion = block.declared_occlusion(offset, shape);
                        &occlusion
                    };
                    let key: Vec<_> = shape
                        .boxes()
                        .iter()
                        .map(|bounds| {
                            [
                                bounds.min(Axis::X),
                                bounds.min(Axis::Y),
                                bounds.min(Axis::Z),
                                bounds.max(Axis::X),
                                bounds.max(Axis::Y),
                                bounds.max(Axis::Z),
                            ]
                            .map(f64::to_bits)
                        })
                        .collect();
                    let id = *encoded.entry(key).or_insert_with(|| {
                        entries.push(encode_shape(shape));
                        entries.len() as i32 - 1
                    });
                    indices.push((block.rename.first as usize + offset, id));
                }
            }
        }
        // Variant references live in the same checked per-state table as the defaults.
        // No runtime content scan, callback, or world-dependent registration is needed.
        let mut contexts = Vec::new();
        for block in &self.blocks {
            if block.context_shapes.is_empty() {
                continue;
            }
            let mut encoded = BTreeMap::new();
            for offset in 0..block.order.len() {
                let mut row = Compound::new();
                row.insert(
                    "state".into(),
                    Tag::Int((block.rename.first as usize + offset) as i32),
                );
                for (channel, variants) in &block.context_shapes {
                    let shape = &variants[offset];
                    let key: Vec<_> = shape
                        .boxes()
                        .iter()
                        .map(|bounds| {
                            [
                                bounds.min(Axis::X),
                                bounds.min(Axis::Y),
                                bounds.min(Axis::Z),
                                bounds.max(Axis::X),
                                bounds.max(Axis::Y),
                                bounds.max(Axis::Z),
                            ]
                            .map(f64::to_bits)
                        })
                        .collect();
                    let id = *encoded.entry(key).or_insert_with(|| {
                        entries.push(encode_shape(shape));
                        entries.len() as i32 - 1
                    });
                    row.insert(channel.as_str().into(), Tag::Int(id));
                }
                contexts.push(Tag::Compound(row));
            }
        }
        if !contexts.is_empty() {
            root.insert("context_shapes".into(), Tag::List(contexts));
        }
        if collision {
            let mut offsets = Vec::new();
            for block in &self.blocks {
                let mut channels = 0;
                if block.shapes.is_some() || block.collision_shapes.is_some() {
                    channels |= 1;
                }
                if block
                    .context_shapes
                    .keys()
                    .any(|key| key.starts_with("collision_"))
                {
                    channels |= 2;
                }
                if block
                    .context_shapes
                    .keys()
                    .any(|key| key.starts_with("outline_"))
                {
                    channels |= 4;
                }
                if block.context_shapes.contains_key("interaction") {
                    channels |= 8;
                }
                if channels == 0 {
                    continue;
                }
                for (index, limits) in block.offset_limits.iter().enumerate() {
                    if let Some((horizontal, vertical)) = limits {
                        offsets.push(Tag::compound([
                            (
                                "state",
                                Tag::Int((block.rename.first as usize + index) as i32),
                            ),
                            ("channels", Tag::Byte(channels)),
                            ("horizontal", Tag::Float(horizontal.to_bits())),
                            ("vertical", Tag::Float(vertical.to_bits())),
                        ]));
                    }
                }
            }
            if !offsets.is_empty() {
                root.insert("shape_offsets".into(), Tag::List(offsets));
            }
        }
        let Some(Tag::IntArray(mapping)) = root.get_mut(&JavaString::from("cached_shapes")) else {
            return Err("collision table has no mapping".into());
        };
        for (state, id) in indices {
            mapping[state] = id;
        }
        Ok(())
    }

    /// The creative inventory (`creative-inventory.json`): each mod tab after
    /// vanilla's, ten to a page (the top row's first
    /// five columns, then the bottom row's), with only the items the game has;
    /// the mods' new items join the search tab under their English names.
    fn creative_tabs(&self, doc: &mut J) -> Result<()> {
        let Some(J::Arr(tabs)) = doc.at("tabs") else {
            return Err("no tabs".into());
        };
        let stack = |id: &str| J::Str(format!("{{count:1,id:\"{id}\"}}"));
        // Blocks' items are among `items` already.
        let new: Vec<&str> = self.items.iter().map(|i| i.rename.to.as_str()).collect();
        let search = tabs
            .iter()
            .position(|t| matches!(t.get("type"), Some(J::Str(k)) if k == "SEARCH"));
        let known = |id: &str| new.contains(&id) || self.vanilla_items.contains(id);
        for (i, tab) in self.tabs.iter().enumerate() {
            let slot = i % 10;
            let mut row = J::Obj(Vec::new());
            row.set("id", J::Str(tab.id.clone()));
            row.set("type", J::Str("CATEGORY".into()));
            row.set(
                "row",
                J::Str(if slot < 5 { "TOP" } else { "BOTTOM" }.into()),
            );
            row.set("column", J::Num((slot % 5).into()));
            row.set("page", J::Num((1 + i / 10).into()));
            row.set("title", J::Str(tab.title.clone()));
            row.set("icon", stack(&tab.icon));
            row.set(
                "build",
                J::Arr(
                    tab.items
                        .iter()
                        .filter(|id| known(id))
                        .map(|id| {
                            J::Obj(vec![
                                ("kind".into(), J::Str("stack".into())),
                                ("visibility".into(), J::Str("PARENT_AND_SEARCH_TABS".into())),
                                ("snbt".into(), stack(id)),
                            ])
                        })
                        .collect(),
                ),
            );
            tabs.push(row);
        }
        if let Some(search) = search {
            let name = |id: &str| {
                let (ns, path) = id.split_once(':').unwrap_or(("", id));
                ["item", "block"]
                    .iter()
                    .find_map(|kind| self.names.get(&format!("{kind}.{ns}.{path}")))
                    .cloned()
                    .unwrap_or_else(|| path.to_owned())
            };
            if let Some(J::Arr(metadata)) = tabs[search].get_mut("search_metadata") {
                metadata.extend(new.into_iter().map(|id| {
                    J::Obj(vec![
                        ("stack".into(), stack(id)),
                        ("lines".into(), J::Arr(vec![J::Str(name(id))])),
                        ("ungrouped".into(), J::Bool(true)),
                    ])
                }));
            }
        }
        Ok(())
    }
    /// One NBT table: `states` lists, `cached_shapes` arrays and `blocks`
    /// compounds, wherever they are.
    fn nbt(&self, tag: &mut Tag) {
        let Tag::Compound(c) = tag else {
            return;
        };
        c.for_each_mut(|key, value| {
            match (key.to_string_lossy().as_str(), value) {
                ("states", Tag::List(list)) if list.len() == self.states => {
                    for b in &self.blocks {
                        // One row per template state per combination, the same
                        // order `states` numbers them in (`Rename::id`): a
                        // template state's combinations are one run, and the
                        // first of the run is where the template state maps.
                        for (id, combo) in b.state_combinations() {
                            let mut state = list[id as usize].clone();
                            if let Some(slot) = state.get_mut("id") {
                                *slot = Tag::String(b.decl.id.as_str().into());
                            }
                            set_nbt_properties(&mut state, combo);
                            list.push(state);
                        }
                    }
                }
                ("cached_shapes", Tag::IntArray(a)) if a.len() == self.states => {
                    for b in &self.blocks {
                        // Parallel to `states`: every combination of a template
                        // state keeps that state's shape.
                        let mut copy = vec![0; b.order.len()];
                        for (at, id) in b.rename.states.clone().enumerate() {
                            for combination in 0..b.rename.fanout {
                                copy[b.rename.row(at as u32, combination) as usize] =
                                    a[id as usize];
                            }
                        }
                        a.extend(copy);
                    }
                }
                ("blocks", Tag::Compound(map)) => {
                    for b in &self.blocks {
                        let Some(mut v) = map
                            .get(&JavaString::from(b.decl.template.as_str()))
                            .cloned()
                        else {
                            continue;
                        };
                        if let Tag::Byte(flags) = &mut v {
                            if let Some(can_occlude) = b.decl.properties.occludes {
                                *flags = (*flags & !16) | if can_occlude { 16 } else { 0 };
                            }
                            if let Some(replaceable) = b.decl.properties.replaceable {
                                // `BlockPropertiesRegistry` reads replaceable from bit 2.
                                *flags = (*flags & !4) | (i8::from(replaceable) << 2);
                            }
                        }
                        if let (Some(f), Some(slot)) =
                            (b.decl.properties.friction, v.get_mut("friction"))
                        {
                            *slot = Tag::Float(f.to_bits());
                        }
                        map.insert(b.decl.id.as_str().into(), v);
                    }
                }
                (_, inner @ Tag::Compound(_)) => self.nbt(inner),
                _ => {}
            }
        });
    }
}
