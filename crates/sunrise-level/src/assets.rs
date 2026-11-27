//! Portable block registry and state-property tables loaded from prepared resources.
use crate::Result;
use serde_json::Value;
use std::{
    collections::BTreeSet,
    io,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering},
    },
};
use sunrise_world::{
    DATA_VERSION,
    heightmap::HeightmapState,
    registry::{BlockRegistry, BlockStateId},
    worldgen::{tags::TagDefinitions, terrain::TerrainBlockStates},
};

pub const MATERIALS: [&str; 6] = ["bedrock", "dirt", "stone", "sand", "oak_planks", "bricks"];

/// Bits of the block states' global palette: 16 for vanilla's 35,723 states
/// (what vanilla servers and clients send), more when content mods add states
/// past 65,536 (docs/modpacks.md). Set once, before the first chunk.
static BLOCK_BITS: AtomicU8 = AtomicU8::new(16);
/// States in the registry, set with [`BLOCK_BITS`].
static BLOCK_STATES: AtomicU32 = AtomicU32::new(u32::MAX);
/// Generation counter incremented whenever [`set_block_states`] updates the palette width.
static BLOCK_STATES_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Sizes the block states' global palette for a registry of `states`.
pub fn set_block_states(states: usize) {
    let bits = sunrise_core::paletted::ceil_log2(states).max(16);
    BLOCK_BITS.store(bits, Ordering::Relaxed);
    BLOCK_STATES.store(u32::try_from(states).unwrap_or(u32::MAX), Ordering::Relaxed);
    BLOCK_STATES_GENERATION.fetch_add(1, Ordering::AcqRel);
}

/// Global block-state palette bit width set by [`set_block_states`].
pub fn block_palette_bits() -> u8 {
    BLOCK_BITS.load(Ordering::Relaxed)
}

/// Number of block states in the active registry set by [`set_block_states`].
pub fn block_states_count() -> u32 {
    BLOCK_STATES.load(Ordering::Relaxed)
}

/// Monotonic generation bumped on every [`set_block_states`] update.
pub fn block_states_generation() -> u64 {
    BLOCK_STATES_GENERATION.load(Ordering::Acquire)
}

#[derive(Clone, Copy)]
pub struct StateFacts {
    air: bool,
    fluid: bool,
    air_block: bool,
    motion: bool,
    no_leaves: bool,
}
impl HeightmapState for StateFacts {
    fn is_air(self) -> bool {
        self.air
    }
    fn is_air_block(self) -> bool {
        self.air_block
    }
    fn has_fluid(self) -> bool {
        self.fluid
    }
    fn blocks_motion_in_heightmap(self) -> bool {
        self.motion
    }
    fn blocks_motion_in_heightmap_no_leaves(self) -> bool {
        self.no_leaves
    }
}
#[derive(Clone)]
pub struct Properties(Arc<Vec<StateFacts>>);
#[cfg(any(test, feature = "test-support"))]
impl Properties {
    /// Heightmap facts for tests, per state: (air, has a fluid, blocks
    /// motion, blocks motion but is not leaves). State 0 is minecraft:air.
    pub fn for_tests(states: &[(bool, bool, bool, bool)]) -> Self {
        Self(Arc::new(
            states
                .iter()
                .enumerate()
                .map(|(i, &(air, fluid, motion, no_leaves))| StateFacts {
                    air,
                    fluid,
                    air_block: i == 0,
                    motion,
                    no_leaves,
                })
                .collect(),
        ))
    }
}
impl TerrainBlockStates for Properties {
    type State = StateFacts;
    fn heightmap_state(&self, id: BlockStateId) -> StateFacts {
        self.0[id.0 as usize]
    }
}
/// The block registry and the facts read from `facts.json` and the block tags:
/// everything world generation and collisions need, with none of the block
/// models or baked sprites.
pub struct Assets {
    /// Leaked once per process: shared by world generation threads.
    pub registry: &'static BlockRegistry,
    pub properties: Properties,
    pub material_states: [BlockStateId; 6],
    /// Block names in `#minecraft:blocks_fluid_flow` (fluid flow vectors).
    pub blocks_fluid_flow: BTreeSet<String>,
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

impl Assets {
    pub fn load(root: &Path) -> Result<Self> {
        let manifest: Value = sunrise_world::json_file::read(&root.join("sunrise-resources.json"))?;
        if manifest["data_version"].as_i64() != Some(i64::from(DATA_VERSION)) {
            return Err(invalid("resources do not match Sunrise's pinned data version").into());
        }
        let (registry, flags) = sunrise_world::level::data::load_registry_with_flags(root)?;
        let bundle: Value = sunrise_world::json_file::read(&root.join("block-tags.json"))?;
        let mut definitions = TagDefinitions::default();
        for (name, text) in bundle
            .as_object()
            .ok_or_else(|| invalid("invalid block tag bundle"))?
        {
            definitions.apply_json(
                name,
                "vanilla",
                text.as_str()
                    .ok_or_else(|| invalid("invalid block tag text"))?,
            )?;
        }
        let built =
            definitions.build(|name, _| registry.default_state(name).map(|_| name.to_owned()))?;
        if !built.missing.is_empty() {
            return Err(invalid("unresolved block tags").into());
        }
        let motion: BTreeSet<_> = built
            .tags
            .get("minecraft:blocks_motion_in_heightmap")
            .ok_or_else(|| invalid("missing motion tag"))?
            .iter()
            .map(String::as_str)
            .collect();
        let no_leaves: BTreeSet<_> = built
            .tags
            .get("minecraft:blocks_motion_in_heightmap_no_leaves")
            .ok_or_else(|| invalid("missing no-leaves motion tag"))?
            .iter()
            .map(String::as_str)
            .collect();
        let blocks_fluid_flow = built
            .tags
            .get("minecraft:blocks_fluid_flow")
            .ok_or_else(|| invalid("missing fluid flow tag"))?
            .iter()
            .cloned()
            .collect();
        let properties = Properties(Arc::new(
            flags
                .iter()
                .enumerate()
                .map(|(i, &(air, fluid))| {
                    let name = &registry
                        .state(BlockStateId(i as u32))
                        .ok_or_else(|| invalid("missing registry state"))?
                        .name;
                    Ok(StateFacts {
                        air,
                        fluid,
                        air_block: name == "minecraft:air",
                        motion: motion.contains(name.as_str()),
                        no_leaves: no_leaves.contains(name.as_str()),
                    })
                })
                .collect::<Result<_>>()?,
        ));
        let mut material_states = [BlockStateId(0); 6];
        for (layer, name) in MATERIALS.iter().enumerate() {
            material_states[layer] = registry
                .default_state(&format!("minecraft:{name}"))
                .ok_or_else(|| invalid("missing test-scene material"))?;
        }
        // Chunks hold every state of this registry: content mods can take it
        // past vanilla's 16-bit global palette.
        set_block_states(registry.len());
        let registry = registry.intern(mod_api::__private::registry_installation_identity());
        Ok(Self {
            registry,
            properties,
            material_states,
            blocks_fluid_flow,
        })
    }
}
