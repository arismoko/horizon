use super::*;

/// A record's `free_tickets` from the POI file the chunk had, while the same
/// POI is still there: a bed or job site a villager claimed stays claimed.
pub(super) fn keep_claims(saved: &mut NamedTag, old: &NamedTag) {
    let key = |r: &Tag| match (r.get("pos"), text(r.get("type"))) {
        (Some(Tag::IntArray(pos)), Some(kind)) => Some((pos.clone(), kind)),
        _ => None,
    };
    let mut claims = HashMap::new();
    if let Some(Tag::Compound(sections)) = old.tag.get("Sections") {
        for section in sections.values() {
            if let Some(Tag::List(records)) = section.get("Records") {
                for r in records {
                    if let Some(k) = key(r) {
                        claims.insert(k, number(r.get("free_tickets")).unwrap_or(0) as i32);
                    }
                }
            }
        }
    }
    let Tag::Compound(root) = &mut saved.tag else {
        return;
    };
    let Some(Tag::Compound(sections)) = root.get_mut(&"Sections".into()) else {
        return;
    };
    sections.for_each_mut(|_, section| {
        let Tag::Compound(section) = section else {
            return;
        };
        let Some(Tag::List(records)) = section.get_mut(&"Records".into()) else {
            return;
        };
        for r in records {
            let Some(&tickets) = key(r).and_then(|k| claims.get(&k)) else {
                continue;
            };
            if let Tag::Compound(r) = r {
                if tickets == 0 {
                    r.remove(&"free_tickets".into());
                } else {
                    r.insert("free_tickets".into(), Tag::Int(tickets));
                }
            }
        }
    });
}
pub(super) fn poi_root(sections: Compound) -> NamedTag {
    NamedTag {
        name: "".into(),
        tag: Tag::compound([
            ("DataVersion", Tag::Int(DATA_VERSION)),
            ("Sections", Tag::Compound(sections)),
        ]),
    }
}

/// A column's sections as `Heightmap.primeHeightmaps` reads them.
struct HeightmapView<'a> {
    sections: &'a [Arc<Section>],
    /// The minimum Y of `sections[0]`.
    column_min_y: i32,
    tables: &'a SaveTables,
    min_y: i32,
    height: i32,
}
impl HeightmapChunk for HeightmapView<'_> {
    type State = crate::assets::StateFacts;
    fn min_y(&self) -> i32 {
        self.min_y
    }
    fn height(&self) -> i32 {
        self.height
    }
    fn highest_section_y(&self) -> i32 {
        let top = self
            .sections
            .iter()
            .rposition(|s| !s.is_uniform(self.tables.air))
            .unwrap_or(0);
        self.column_min_y + top as i32 * 16
    }
    fn block_state(&self, x: i32, y: i32, z: i32) -> Self::State {
        // Chunk.get: no block outside the sections.
        let state = usize::try_from((y - self.column_min_y) >> 4)
            .ok()
            .and_then(|s| self.sections.get(s))
            .map_or(0, |s| s.get(crate::chunk::index(x, y, z)));
        self.tables.heightmaps.heightmap_state(BlockStateId(state))
    }
}
/// Heightmaps kept after world generation (`ChunkStatus.FULL.heightmapsAfter`).
fn heightmaps(chunk: &Chunk, tables: &SaveTables, shape: LevelShape) -> Tag {
    let kinds = [
        HeightmapType::WorldSurface,
        HeightmapType::OceanFloor,
        HeightmapType::MotionBlocking,
        HeightmapType::MotionBlockingNoLeaves,
    ];
    let mut maps: Vec<Heightmap> = kinds
        .iter()
        .filter_map(|k| Heightmap::new(*k, shape.min_y, shape.height).ok())
        .collect();
    let view = HeightmapView {
        sections: &chunk.sections,
        column_min_y: chunk.min_y(),
        tables,
        min_y: shape.min_y,
        height: shape.height,
    };
    let mut refs: Vec<&mut Heightmap> = maps.iter_mut().collect();
    if Heightmap::prime(&view, &mut refs).is_err() {
        return Tag::Compound(Compound::new());
    }
    Tag::compound(maps.iter().map(|m| {
        (
            m.kind().name(),
            Tag::LongArray(m.raw_data().iter().map(|&w| w as i64).collect()),
        )
    }))
}
/// The client heightmaps of a column's sections (from `column_min_y`), in a
/// level from `min_y`, `height` blocks high.
pub fn client_heightmaps_in(
    sections: &[Arc<Section>],
    column_min_y: i32,
    tables: &SaveTables,
    min_y: i32,
    height: i32,
) -> Vec<(i32, Vec<i64>)> {
    let kinds = [
        (1, HeightmapType::WorldSurface),
        (4, HeightmapType::MotionBlocking),
        (5, HeightmapType::MotionBlockingNoLeaves),
    ];
    let mut maps: Vec<(i32, Heightmap)> = kinds
        .iter()
        .filter_map(|(id, k)| Heightmap::new(*k, min_y, height).ok().map(|m| (*id, m)))
        .collect();
    let view = HeightmapView {
        sections,
        column_min_y,
        tables,
        min_y,
        height,
    };
    let mut refs: Vec<&mut Heightmap> = maps.iter_mut().map(|(_, m)| m).collect();
    if Heightmap::prime(&view, &mut refs).is_err() {
        return Vec::new();
    }
    maps.iter()
        .map(|(id, m)| (*id, m.raw_data().iter().map(|&w| w as i64).collect()))
        .collect()
}
/// Vanilla light layer (nibbles, index y<<8|z<<4|x, low nibble first).
pub(super) fn nibbles(light: &Light, sky: bool) -> Vec<u8> {
    let mut out = vec![0u8; 2048];
    for (i, byte) in out.iter_mut().enumerate() {
        let value = |j: usize| {
            let v = light.get(j);
            if sky { v >> 4 } else { v & 15 }
        };
        *byte = value(2 * i) | value(2 * i + 1) << 4;
    }
    out
}

pub fn encode(
    chunk: &Chunk,
    registry: &BlockRegistry,
    biomes: &Biomes,
    items: &crate::items::Items,
) -> NamedTag {
    encode_with(chunk, registry, biomes, None, LevelShape::OVERWORLD, items)
}
/// A save's lookup of an old chunk's calculated blending data.
pub type BlendingLookup = Box<dyn Fn(i32, i32) -> Option<Arc<BlendingData>> + Send + Sync>;
/// [`encode_with`] for a save of `SaveDir`: an old chunk whose blending data
/// the generation calculated saves it (`BlendingData.pack`), when the record
/// it was loaded from has the same section range; otherwise `blending_data`
/// stays as it was loaded.
pub(super) fn encode_saved(
    chunk: &Chunk,
    registry: &BlockRegistry,
    biomes: &Biomes,
    tables: Option<&SaveTables>,
    shape: LevelShape,
    blending: Option<&BlendingData>,
    items: &crate::items::Items,
) -> std::result::Result<NamedTag, sunrise_mod_runtime::attachments::AttachmentEncodeError> {
    let mut root = encode_full(
        chunk,
        registry,
        biomes,
        tables,
        shape,
        items,
        ChunkEncodingEdits::default(),
    )?;
    if let Some(data) = blending {
        put_calculated_blending(&mut root, data);
    }
    Ok(root)
}
pub(super) fn put_calculated_blending(root: &mut NamedTag, data: &BlendingData) {
    let Tag::Compound(map) = &mut root.tag else {
        return;
    };
    let Some(saved) = map.get(&"blending_data".into()) else {
        return;
    };
    let (min_section, max_section, heights) = data.pack();
    if saved.get("min_section") != Some(&Tag::Int(min_section))
        || saved.get("max_section") != Some(&Tag::Int(max_section))
    {
        return;
    }
    let mut fields = vec![
        ("min_section", Tag::Int(min_section)),
        ("max_section", Tag::Int(max_section)),
    ];
    if let Some(heights) = heights {
        fields.push((
            "heights",
            Tag::List(heights.iter().map(|h| Tag::Float(h.to_bits())).collect()),
        ));
    }
    map.insert("blending_data".into(), Tag::compound(fields));
}
/// The chunk NBT vanilla's `SerializableChunkData.write` produces for a full
/// chunk in a level of `shape`.
pub fn encode_with(
    chunk: &Chunk,
    registry: &BlockRegistry,
    biomes: &Biomes,
    tables: Option<&SaveTables>,
    shape: LevelShape,
    items: &crate::items::Items,
) -> NamedTag {
    encode_full(
        chunk,
        registry,
        biomes,
        tables,
        shape,
        items,
        ChunkEncodingEdits::default(),
    )
    .expect("chunk attachments encode cleanly")
}

pub(super) struct ChunkEncodingEdits<'a> {
    pub(super) block_entities: bool,
    pub(super) attachments: Option<&'a Compound>,
}

impl Default for ChunkEncodingEdits<'_> {
    fn default() -> Self {
        Self {
            block_entities: true,
            attachments: None,
        }
    }
}

pub(super) fn encode_full(
    chunk: &Chunk,
    registry: &BlockRegistry,
    biomes: &Biomes,
    tables: Option<&SaveTables>,
    shape: LevelShape,
    items: &crate::items::Items,
    edits: ChunkEncodingEdits<'_>,
) -> std::result::Result<NamedTag, sunrise_mod_runtime::attachments::AttachmentEncodeError> {
    let sky_light = shape.sky_light;
    let mut sections = Vec::new();
    // Sky layers below the highest section with light other than 15 are
    // written (even all-zero ones): vanilla derives missing layers from the
    // layer above them.
    let top_sky = chunk
        .sections
        .iter()
        .rposition(|s| s.light.any_not(0xF0, 0xF0));
    // `SerializableChunkData.copyOf` checks the engine's whole light range, not just
    // sections with block data. A dimension can be narrower than the chunk's allocation.
    let light_y = shape.min_section() - 1..=(shape.max_y() >> 4);
    let first = (chunk.min_y() >> 4) - 1;
    for y in light_y {
        let b = (y - first) as usize;
        let section = match b {
            0 => chunk.pad_below.as_ref(),
            _ if b == chunk.sections.len() + 1 => chunk.pad_above.as_ref(),
            _ => chunk.sections.get(b - 1),
        };
        let Some(section) = section else { continue };
        let layers = if chunk.lit {
            if let Some(stored) = chunk.stored_light() {
                let [sky, block] = stored.presence(b);
                if let Some([sky_array, block_array]) = stored.saved_arrays(b) {
                    // `SerializableChunkData.copyOf`: omit empty layers restored from
                    // section status, but keep materialized arrays, including zero arrays.
                    [
                        sky && (sky_array || section.light.any(0xF0)),
                        block && (block_array || section.light.any(15)),
                    ]
                } else {
                    [sky, block]
                }
            } else if shape.has_section(y) {
                [
                    top_sky.is_some_and(|top| b <= top + 2),
                    section.light.any(15),
                ]
            } else {
                [false; 2]
            }
        } else {
            [false; 2]
        };
        let mut entries = vec![("Y", Tag::Byte(y as i8))];
        if layers[1] {
            entries.push(("BlockLight", Tag::ByteArray(nibbles(&section.light, false))));
        }
        if sky_light && layers[0] {
            entries.push(("SkyLight", Tag::ByteArray(nibbles(&section.light, true))));
        }
        if !shape.has_section(y) {
            if entries.len() > 1 {
                sections.push(Tag::compound(entries));
            }
            continue;
        }
        // PalettedContainer.pack: distinct states in index order, indices at
        // the saved width.
        let packed = section.blocks.pack();
        let mut states = vec![(
            "palette",
            Tag::List(
                packed
                    .palette
                    .iter()
                    .map(|&id| {
                        let state = registry.state(BlockStateId(id));
                        match state {
                            // BlockState.CODEC: the block name for default states.
                            Some(s)
                                if tables.is_some_and(|t| {
                                    t.default_state.get(id as usize) == Some(&true)
                                }) =>
                            {
                                string(&s.name)
                            }
                            Some(s) => s.to_nbt(),
                            None => string("minecraft:air"),
                        }
                    })
                    .collect(),
            ),
        )];
        if let Some(data) = &packed.data {
            states.push((
                "data",
                Tag::LongArray(data.iter().map(|&w| w as i64).collect()),
            ));
        }
        let biome_packed = section.biomes.pack();
        let mut biome_tag = vec![(
            "palette",
            Tag::List(
                biome_packed
                    .palette
                    .iter()
                    .map(|&b| string(&biomes.saved_name(b as u16)))
                    .collect(),
            ),
        )];
        if let Some(data) = &biome_packed.data {
            biome_tag.push((
                "data",
                Tag::LongArray(data.iter().map(|&w| w as i64).collect()),
            ));
        }
        entries.extend([
            ("block_states", Tag::compound(states)),
            ("biomes", Tag::compound(biome_tag)),
        ]);
        sections.push(Tag::compound(entries));
    }
    let extras = chunk.saved.as_deref();
    let block_name = |p: &(i32, i32, i32)| -> String {
        registry
            .state(chunk.block(p.0 & 15, p.1, p.2 & 15))
            .map_or_else(String::new, |s| s.name.clone())
    };
    let mut block_entities: Vec<Tag> = chunk
        .block_entities
        .iter()
        .map(|(p, e)| {
            let block = block_name(p);
            let stored = extras
                .and_then(|x| x.block_entity_sources.iter().find(|(q, _)| q == p))
                .map(|(_, tag)| tag);
            let source = stored.filter(|tag| {
                crate::block_entity_nbt::id(tag).as_deref() == Some(block_entity_id(e, &block))
            });
            let pending = extras.and_then(|x| x.pending_loot.get(p));
            let lock = source.and_then(|tag| tag.get("lock"));
            let hopper_cooldown = (block == "minecraft:hopper").then(|| {
                source
                    .and_then(|tag| number(tag.get("TransferCooldown")))
                    .unwrap_or(-1) as i32
            });
            let current = block_entity_encode::encode_with(
                *p,
                e,
                &block,
                block_entity_encode::DataFields {
                    lock,
                    custom_name: chunk
                        .block_names
                        .iter()
                        .find(|(q, _)| q == p)
                        .map(|(_, name)| name),
                    pending,
                    hopper_cooldown,
                },
                items,
                |_| {
                    if edits.block_entities {
                        extras?.block_entity_save_edits.as_ref()?.get(p).cloned()
                    } else {
                        None
                    }
                },
            );
            let mut saved = match source {
                Some(original) => overlay_block_entity_source(original, e, &block, &current, items),
                None => current,
            };
            // A consumed loot table must not be restored from an opaque source.
            let chest = matches!(e, crate::containers::BlockEntity::Chest(_));
            if chest
                && pending.is_none()
                && let Tag::Compound(map) = &mut saved
            {
                map.remove(&"LootTable".into());
                map.remove(&"LootTableSeed".into());
            }
            // The overlay may restore an unchanged stack's original patch. Apply
            // catalog validation without rebuilding the Java table when replacing or
            // dropping fields. If normalization supplies missing defaults, use its
            // fresh field order, including the common metadata tail.
            let live_items = (chest && pending.is_some() && e.slots().iter().any(Option::is_some))
                .then(|| encode_items(e.slots(), items));
            if let Some(mut canonical) = crate::block_entity_nbt::canonical_with_stack(
                &saved,
                &|tag| {
                    crate::block_entity_nbt::stack(tag)
                        .and_then(|tag| item_from_nbt(&tag))
                        .map(|stack| item_to_nbt(&stack, items))
                },
                live_items,
            ) {
                if let Tag::Compound(map) = &mut saved
                    && canonical.keys().all(|key| map.contains_key(key))
                {
                    map.retain(|key, value| {
                        if let Some(admitted) = canonical.remove(key) {
                            *value = admitted;
                            true
                        } else {
                            false
                        }
                    });
                } else {
                    saved = Tag::Compound(canonical);
                }
            }
            if let Some(limit) = stored.and_then(|tag| tag.get("sunrise:stack_limit"))
                && let Tag::Compound(map) = &mut saved
            {
                map.insert("sunrise:stack_limit".into(), limit.clone());
            }
            if let Some((_, attachments)) =
                extras.and_then(|e| e.block_entity_attachments.iter().find(|(pos, _)| pos == p))
                && let Tag::Compound(map) = &mut saved
                && attachment_compound(map.get(&ATTACHMENTS_KEY.into())).is_ok()
            {
                let data = attachments.try_to_nbt()?;
                if data.is_empty() {
                    map.remove(&ATTACHMENTS_KEY.into());
                } else {
                    map.insert(ATTACHMENTS_KEY.into(), Tag::Compound(data));
                }
            }
            Ok(saved)
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if let Some(extras) = extras {
        // A block caught mid-move whose piston record could not be written carries its
        // block entity's attachments here (`Game::attach_save_data`) with no live block
        // entity to write them: one that cannot encode refuses the whole record, so the
        // old record stays on disk instead of the piston going missing.
        for (pos, attachments) in &extras.block_entity_attachments {
            if !chunk.block_entities.iter().any(|(p, _)| p == pos) {
                attachments.try_to_nbt()?;
            }
        }
        for raw in &extras.block_entities {
            let pos = (
                number(raw.get("x")).unwrap_or(0) as i32,
                number(raw.get("y")).unwrap_or(0) as i32,
                number(raw.get("z")).unwrap_or(0) as i32,
            );
            let state = chunk.get(pos.0 & 15, pos.1, pos.2 & 15) as usize;
            let still_there =
                tables.is_none_or(|t| t.block_entity.get(state).copied().unwrap_or(true));
            if still_there && !chunk.block_entities.iter().any(|(p, _)| *p == pos) {
                block_entities
                    .push(crate::block_entity_nbt::canonical(raw).unwrap_or_else(|| raw.clone()));
            }
        }
    }
    let mut root = extras
        .map(|e| Tag::Compound(e.root.clone()))
        .unwrap_or_else(|| Tag::Compound(Compound::new()));
    for (key, value) in [
        ("DataVersion", Tag::Int(DATA_VERSION)),
        ("xPos", Tag::Int(chunk.x)),
        ("zPos", Tag::Int(chunk.z)),
        ("yPos", Tag::Int(shape.min_section())),
        ("Status", string("minecraft:full")),
        ("LastUpdate", Tag::Long(extras.map_or(0, |e| e.last_update))),
        ("isLightOn", Tag::Byte(i8::from(chunk.lit))),
        ("sections", Tag::List(sections)),
        ("block_entities", Tag::List(block_entities)),
    ] {
        put(&mut root, key, value);
    }
    if let Some(Tag::Compound(map)) = Some(&mut root) {
        let attachments = if let Some(attachments) = edits.attachments {
            Some(attachments.clone())
        } else if let Some(extras) = extras
            && attachment_compound(map.get(&ATTACHMENTS_KEY.into())).is_ok()
        {
            Some(extras.attachments.try_to_nbt()?)
        } else {
            None
        };
        if let Some(attachments) = attachments {
            if attachments.is_empty() {
                map.remove(&ATTACHMENTS_KEY.into());
            } else {
                map.insert(ATTACHMENTS_KEY.into(), Tag::Compound(attachments));
            }
        }
        map.entry("InhabitedTime".into()).or_insert(Tag::Long(0));
        map.entry("structures".into()).or_insert_with(|| {
            Tag::compound([
                ("References", Tag::Compound(Compound::new())),
                ("starts", Tag::Compound(Compound::new())),
            ])
        });
        if !chunk.structure_refs.is_empty()
            && let Some(Tag::Compound(s)) = map.get_mut(&"structures".into())
        {
            let mut refs: std::collections::BTreeMap<&str, Vec<i64>> = Default::default();
            for (id, (x, z)) in &chunk.structure_refs {
                refs.entry(id)
                    .or_default()
                    .push(i64::from(*x as u32) | i64::from(*z) << 32);
            }
            let refs: Compound = refs
                .into_iter()
                .map(|(id, v)| (id.into(), Tag::LongArray(v)))
                .collect();
            s.insert("References".into(), Tag::Compound(refs));
        }
        let starts: Compound = chunk
            .structure_starts
            .iter()
            .filter_map(|start| match start.save() {
                Some(tag) => Some((start.name.as_str().into(), tag)),
                None => {
                    log::warn!(
                        "Sunrise: structure start {} not saved: one of its pieces cannot be",
                        start.name
                    );
                    None
                }
            })
            .collect();
        if !starts.is_empty()
            && let Some(Tag::Compound(s)) = map.get_mut(&"structures".into())
        {
            s.insert("starts".into(), Tag::Compound(starts));
        }
        map.entry("PostProcessing".into())
            .or_insert_with(|| Tag::List(vec![Tag::List(Vec::new()); shape.section_count()]));
    }
    if let Some(t) = tables {
        put(&mut root, "Heightmaps", heightmaps(chunk, t, shape));
        let names = |k: u32| t.block_names.get(k as usize).cloned();
        let (blocks, fluids) = extras.map_or((&[][..], &[][..]), |e| {
            (&e.block_ticks[..], &e.fluid_ticks[..])
        });
        put(&mut root, "block_ticks", tick_list(blocks, names));
        put(&mut root, "fluid_ticks", tick_list(fluids, fluid_name));
    }
    Ok(NamedTag {
        name: "".into(),
        tag: root,
    })
}
/// Root keys `encode_with` regenerates; everything else is kept as loaded.
const GENERATED_KEYS: [&str; 14] = [
    "DataVersion",
    "xPos",
    "zPos",
    "yPos",
    "Status",
    "LastUpdate",
    "isLightOn",
    "sections",
    "block_entities",
    "Heightmaps",
    "block_ticks",
    "fluid_ticks",
    "entities",
    "CarvingMask",
];
pub fn decode(root: &NamedTag, registry: &BlockRegistry, biomes: &Biomes) -> Option<Chunk> {
    decode_with(root, registry, biomes, None, ColumnLayout::VANILLA)
}
/// Decode a full chunk for read-only inspection (`SavedChunkReader`):
/// - Rejects missing, non-string, or non-full `Status` (matching vanilla 26.3
///   `SerializableChunkData.parse`, which rejects missing/empty `Status`).
/// - Rejects records whose `xPos`/`zPos` do not match `expected_pos` (returns `Err`).
/// - Bounds section `Y` with checked arithmetic against `shape`'s actual section range
///   (`min_section..min_section + section_count`), preventing signed overflow panics on
///   malformed NBT.
/// - Skips block-entity and scheduled-tick decoding (read-only snapshots only expose blocks,
///   biomes, heightmaps, and light).
pub fn decode_readonly_full(
    root: &NamedTag,
    expected_pos: (i32, i32),
    registry: &BlockRegistry,
    biomes: &Biomes,
    shape: LevelShape,
) -> std::result::Result<Option<Chunk>, String> {
    let tag = &root.tag;
    let Some(status) = text(tag.get("Status")) else {
        return Err("missing or non-string chunk Status".into());
    };
    if status != "minecraft:full" && status != "full" {
        return Ok(None);
    }
    let Some(Tag::Int(x)) = tag.get("xPos") else {
        return Err("missing or non-int chunk xPos".into());
    };
    let Some(Tag::Int(z)) = tag.get("zPos") else {
        return Err("missing or non-int chunk zPos".into());
    };
    if (*x, *z) != expected_pos {
        return Err(format!(
            "saved chunk coordinates ({x}, {z}) do not match requested ({}, {})",
            expected_pos.0, expected_pos.1
        ));
    }
    let layout = shape.column();
    let min_section = i64::from(shape.min_y >> 4);
    let max_section_excl = min_section + i64::from(shape.section_count() as i32);
    let mut chunk = Chunk::with_layout(*x, *z, layout);
    let Some(air) = registry.default_state("minecraft:air").map(|id| id.0) else {
        return Err("missing minecraft:air in block registry".into());
    };
    if let Some(Tag::List(sections)) = tag.get("sections") {
        for section in sections {
            let Some(y) = number(section.get("Y")) else {
                continue;
            };
            if y < min_section || y >= max_section_excl {
                continue;
            }
            let Some(y_blocks) = i32::try_from(y).ok().and_then(|sy| sy.checked_mul(16)) else {
                continue;
            };
            let Some(si) = layout.section_index(y_blocks) else {
                continue;
            };
            if let Some(states) = section.get("block_states")
                && let Some(Tag::List(palette)) = states.get("palette")
            {
                let palette: Vec<u32> = palette
                    .iter()
                    .map(|p| registry.saved_state_id(p).map_or(air, |id| id.0))
                    .collect();
                let target = Arc::make_mut(&mut chunk.sections[si]);
                let data = match states.get("data") {
                    Some(Tag::LongArray(data)) => {
                        Some(data.iter().map(|&w| w as u64).collect::<Vec<_>>())
                    }
                    _ => None,
                };
                if let Ok(blocks) =
                    PalettedContainer::from_packed(block_strategy(), palette, data.as_deref())
                {
                    target.blocks = blocks;
                }
            }
            if let Some(b) = section.get("biomes")
                && let Some(Tag::List(palette)) = b.get("palette")
            {
                let ids: Vec<u16> = palette
                    .iter()
                    .map(|p| match p {
                        Tag::String(s) => biomes.saved_id(&s.to_string_lossy()),
                        _ => 0,
                    })
                    .collect();
                let data = match b.get("data") {
                    Some(Tag::LongArray(data)) => {
                        Some(data.iter().map(|&w| w as u64).collect::<Vec<_>>())
                    }
                    _ => None,
                };
                let palette: Vec<u32> = ids.iter().map(|&id| u32::from(id)).collect();
                let target = Arc::make_mut(&mut chunk.sections[si]);
                if let Ok(container) =
                    PalettedContainer::from_packed(BIOME_STRATEGY, palette.clone(), data.as_deref())
                {
                    target.biomes = container;
                } else if let Some(&first) = palette.first() {
                    target.biomes = PalettedContainer::new(BIOME_STRATEGY, first);
                }
            }
        }
    }
    Ok(Some(chunk))
}

/// Only full chunks load here; partially generated ones (vanilla's world
/// border of proto chunks) resume generation through `decode_proto`.
pub fn decode_with(
    root: &NamedTag,
    registry: &BlockRegistry,
    biomes: &Biomes,
    tables: Option<&SaveTables>,
    layout: ColumnLayout,
) -> Option<Chunk> {
    if text(root.tag.get("Status")).is_some_and(|s| s != "minecraft:full" && s != "full") {
        return None;
    }
    decode_column(root, registry, biomes, tables, layout)
}
/// `decode_with` whatever the chunk's status, into a chunk of `layout`
/// (sections outside it are left out).
fn decode_column(
    root: &NamedTag,
    registry: &BlockRegistry,
    biomes: &Biomes,
    tables: Option<&SaveTables>,
    layout: ColumnLayout,
) -> Option<Chunk> {
    let tag = &root.tag;
    let Tag::Int(x) = tag.get("xPos")? else {
        return None;
    };
    let Tag::Int(z) = tag.get("zPos")? else {
        return None;
    };
    let mut chunk = Chunk::with_layout(*x, *z, layout);
    // structures.References: structure id -> packed start chunk positions.
    if let Some(Tag::Compound(refs)) = tag.get("structures").and_then(|s| s.get("References")) {
        for (id, v) in refs {
            if let Tag::LongArray(starts) = v {
                let id: Arc<str> = Arc::from(id.to_string_lossy());
                for &p in starts {
                    chunk
                        .structure_refs
                        .push((id.clone(), (p as i32, (p >> 32) as i32)));
                }
            }
        }
    }
    let air = registry.default_state("minecraft:air")?.0;
    let Some(Tag::List(sections)) = tag.get("sections") else {
        return Some(chunk);
    };
    for section in sections {
        let Some(y) = number(section.get("Y")) else {
            continue;
        };
        let Some(si) = layout.section_index(y as i32 * 16) else {
            continue;
        };
        if let Some(states) = section.get("block_states")
            && let Some(Tag::List(palette)) = states.get("palette")
        {
            let palette: Vec<u32> = palette
                .iter()
                .map(|p| registry.saved_state_id(p).map_or(air, |id| id.0))
                .collect();
            let target = Arc::make_mut(&mut chunk.sections[si]);
            let data = match states.get("data") {
                Some(Tag::LongArray(data)) => {
                    Some(data.iter().map(|&w| w as u64).collect::<Vec<_>>())
                }
                _ => None,
            };
            if let Ok(blocks) =
                PalettedContainer::from_packed(block_strategy(), palette, data.as_deref())
            {
                target.blocks = blocks;
            }
        }
        if let Some(b) = section.get("biomes")
            && let Some(Tag::List(palette)) = b.get("palette")
        {
            let ids: Vec<u16> = palette
                .iter()
                .map(|p| match p {
                    Tag::String(s) => biomes.saved_id(&s.to_string_lossy()),
                    _ => 0,
                })
                .collect();
            let data = match b.get("data") {
                Some(Tag::LongArray(data)) => {
                    Some(data.iter().map(|&w| w as u64).collect::<Vec<_>>())
                }
                _ => None,
            };
            let palette: Vec<u32> = ids.iter().map(|&id| u32::from(id)).collect();
            let target = Arc::make_mut(&mut chunk.sections[si]);
            if let Ok(container) =
                PalettedContainer::from_packed(BIOME_STRATEGY, palette.clone(), data.as_deref())
            {
                target.biomes = container;
            } else if let Some(&first) = palette.first() {
                target.biomes = PalettedContainer::new(BIOME_STRATEGY, first);
            }
        }
    }
    let mut extras = ChunkExtras {
        last_update: number(tag.get("LastUpdate")).unwrap_or(0),
        ..ChunkExtras::default()
    };
    if let Some(Tag::List(entities)) = tag.get("block_entities") {
        for e in entities {
            if let Some(at) = block_entity_order::saved_at(e, [*x, *z]) {
                extras.block_entity_order.push(at);
            }
            let int = |k: &str| number(e.get(k)).map(|v| v as i32);
            let decoded = if let (Some(x), Some(y), Some(z)) = (int("x"), int("y"), int("z")) {
                let state = chunk.block(x & 15, y, z & 15);
                registry
                    .state(state)
                    .and_then(|block| decode_block_entity_for_block(e, &block.name))
                    .map(|(_, be)| (block_entity_pos((x, y, z), [chunk.x, chunk.z]), be))
            } else {
                None
            };
            match decoded {
                Some(be) => {
                    if let Ok(Some(data)) = attachment_compound(e.get(ATTACHMENTS_KEY)) {
                        extras.block_entity_attachments.push((
                            be.0,
                            sunrise_mod_runtime::attachments::Attachments::from_nbt(data.clone()),
                        ));
                    }
                    let name = if matches!(
                        be.1,
                        crate::containers::BlockEntity::Banner(_)
                            | crate::containers::BlockEntity::Sign(_)
                            | crate::containers::BlockEntity::Mod(_)
                    ) {
                        text(e.get("CustomName")).map(|name| string(&name))
                    } else {
                        e.get("CustomName")
                            .and_then(crate::block_entity_nbt::component)
                    };
                    if let Some(name) = name {
                        chunk.block_names.push((be.0, name));
                    }
                    let block = registry
                        .state(chunk.block(be.0.0 & 15, be.0.1, be.0.2 & 15))
                        .map_or("", |state| state.name.as_str());
                    let randomizable = randomizable_container(&be.1, block);
                    if randomizable && let Some(pending) = PendingContainerLoot::from_tag(e) {
                        extras.pending_loot.insert(be.0, pending);
                    }
                    let mut source = e.clone();
                    if randomizable && let Tag::Compound(map) = &mut source {
                        map.remove(&"LootTable".into());
                        map.remove(&"LootTableSeed".into());
                    }
                    extras.block_entity_sources.push((be.0, source));
                    chunk.block_entities.push(be);
                }
                None => extras.block_entities.push(e.clone()),
            }
        }
    }
    if let Tag::Compound(map) = tag {
        if let Ok(Some(data)) = attachment_compound(map.get(&ATTACHMENTS_KEY.into())) {
            extras.attachments =
                sunrise_mod_runtime::attachments::Attachments::from_nbt(data.clone());
        }
        extras.root = map
            .iter()
            .filter(|(k, _)| !GENERATED_KEYS.contains(&k.to_string_lossy().as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
    }
    if let Some(t) = tables {
        extras.block_ticks = read_ticks(tag.get("block_ticks"), (*x, *z), |n| {
            t.block_ids.get(n).map(|&i| u32::from(i))
        });
        extras.fluid_ticks = read_ticks(tag.get("fluid_ticks"), (*x, *z), fluid_id);
    }
    chunk.saved = Some(Arc::new(extras));
    Some(chunk)
}

pub fn encode_proto(
    proto: &SavedProto,
    registry: &BlockRegistry,
    biomes: &Biomes,
    tables: Option<&SaveTables>,
    shape: LevelShape,
    items: &crate::items::Items,
) -> NamedTag {
    encode_proto_with_attachments(proto, registry, biomes, tables, shape, items, None)
        .expect("proto chunk attachments encode cleanly")
}

pub(super) fn encode_proto_with_attachments(
    proto: &SavedProto,
    registry: &BlockRegistry,
    biomes: &Biomes,
    tables: Option<&SaveTables>,
    shape: LevelShape,
    items: &crate::items::Items,
    attachments: Option<&Compound>,
) -> std::result::Result<NamedTag, sunrise_mod_runtime::attachments::AttachmentEncodeError> {
    let mut root = encode_full(
        &proto.column,
        registry,
        biomes,
        tables,
        shape,
        items,
        ChunkEncodingEdits {
            block_entities: true,
            attachments,
        },
    )?;
    let status = if proto.decorated {
        ChunkStatus::Features
    } else {
        ChunkStatus::Terrain
    };
    // ProtoChunk.postProcessing: one list per section, packed section offsets.
    let mut marks = vec![Vec::new(); shape.section_count()];
    for &[x, y, z] in &proto.column.postprocess {
        let section = usize::try_from((y - shape.min_y) >> 4).ok();
        if let Some(list) = section.and_then(|s| marks.get_mut(s)) {
            list.push(Tag::Short(
                ((x & 15) | (y & 15) << 4 | (z & 15) << 8) as i16,
            ));
        }
    }
    for (key, value) in [
        ("Status", string(status.name())),
        ("isLightOn", Tag::Byte(0)),
        ("block_entities", Tag::List(proto.block_entities.clone())),
        ("entities", Tag::List(proto.entities.clone())),
        (
            "PostProcessing",
            Tag::List(marks.into_iter().map(Tag::List).collect()),
        ),
    ] {
        put(&mut root.tag, key, value);
    }
    Ok(root)
}
/// A proto chunk generation can resume: `terrain` (written to by a
/// neighbour's decoration) up to `spawn`. Earlier statuses hold nothing that
/// generating the chunk again doesn't reproduce.
pub fn decode_proto(
    root: &NamedTag,
    registry: &BlockRegistry,
    biomes: &Biomes,
    tables: Option<&SaveTables>,
    layout: ColumnLayout,
) -> Option<SavedProto> {
    let status = ChunkStatus::parse(&text(root.tag.get("Status"))?).ok()?;
    if !(ChunkStatus::Terrain..ChunkStatus::Full).contains(&status) {
        return None;
    }
    let mut column = decode_column(root, registry, biomes, tables, layout)?;
    column.block_entities.clear();
    let base = number(root.tag.get("yPos")).map_or(column.min_y(), |y| y as i32 * 16);
    if let Some(Tag::List(sections)) = root.tag.get("PostProcessing") {
        for (si, marks) in sections.iter().enumerate() {
            let Tag::List(marks) = marks else {
                continue;
            };
            for mark in marks {
                if let Tag::Short(p) = mark {
                    let p = i32::from(*p);
                    column.postprocess.push([
                        column.x * 16 + (p & 15),
                        base + si as i32 * 16 + ((p >> 4) & 15),
                        column.z * 16 + ((p >> 8) & 15),
                    ]);
                }
            }
        }
    }
    let list = |key: &str| match root.tag.get(key) {
        Some(Tag::List(list)) => list.clone(),
        _ => Vec::new(),
    };
    Some(SavedProto {
        column,
        decorated: status >= ChunkStatus::Features,
        block_entities: list("block_entities"),
        entities: list("entities"),
    })
}

/// Read the `SkyLight`/`BlockLight` byte arrays of a chunk saved with
/// `isLightOn` (`SerializableChunkData.read`, which queues the arrays via
/// `ThreadedLevelLightEngine.queueSectionData`). The render cache inherits missing
/// sky light from the bottom row of the next stored layer above, or gets full
/// sky above all stored layers. Empty layers omitted by NBT are restored from
/// published section status when the chunk's light is initialized.
pub fn load_light(tag: &Tag, chunk: &mut Chunk, sky_light: bool) {
    if tag.get("isLightOn") != Some(&Tag::Byte(1)) {
        return;
    }
    let Some(Tag::List(sections)) = tag.get("sections") else {
        return;
    };
    let mut layers = vec![(None, None); chunk.sections.len() + 2];
    let first = (chunk.min_y() >> 4) - 1;
    for section in sections {
        let Some(y) = number(section.get("Y")) else {
            continue;
        };
        let Some(slot) = y
            .checked_sub(i64::from(first))
            .and_then(|b| usize::try_from(b).ok())
            .and_then(|b: usize| layers.get_mut(b))
        else {
            continue;
        };
        let layer = |name: &str| match section.get(name) {
            Some(Tag::ByteArray(a)) if a.len() == 2048 => Some(&a[..]),
            _ => None,
        };
        *slot = (layer("SkyLight"), layer("BlockLight"));
    }
    crate::light::light_from_saved_layers(chunk, &layers, sky_light);
}
