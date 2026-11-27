use super::*;
use crate::game::client_side::resource_tables_client::ClientPackResources;
use sunrise_client_render::streaming::WorkerContext;
use sunrise_items::blocks::{MAP_COLORS, RenderShape};
use sunrise_level::particle_options::{ParticleOptions, ParticleType};

impl crate::game::ClientGame<'_> {
    fn particle_atlas_sprite(sprite: String) -> String {
        if sprite == "missingno" {
            sunrise_client::hud::MISSING_SPRITE.into()
        } else {
            sprite
        }
    }

    pub(in crate::game) fn block_provider_options(
        &self,
        ty: ParticleType,
        state: BlockStateId,
        pos: [i32; 3],
    ) -> Option<ParticleOptions> {
        self.common.pack.assets.registry.state(state)?;
        let facts = self.common.res.facts();
        let block = facts.get(state);
        let sprite = || {
            Self::particle_atlas_sprite(
                Self::block_particle_sprite(&self.client.client_pack, state)
                    .unwrap_or_else(|| "missingno".into()),
            )
        };
        let options = match ty {
            ParticleType::Block | ParticleType::DustPillar | ParticleType::BlockCrumble => {
                if facts.is_air(state)
                    || &*block.name == "minecraft:moving_piston"
                    || !block.spawn_terrain_particles
                {
                    return None;
                }
                ParticleOptions::block(
                    ty,
                    sprite(),
                    self.terrain_particle_tint(state, pos).unwrap_or(0xFFFFFF),
                )
            }
            ParticleType::FallingDust => {
                if !facts.is_air(state) && block.render_shape == RenderShape::Invisible {
                    return None;
                }
                let map_color = || MAP_COLORS[usize::from(block.map_color)];
                let tint = match &*block.name {
                    "minecraft:anvil" | "minecraft:chipped_anvil" | "minecraft:damaged_anvil" => {
                        map_color()
                    }
                    name if name.starts_with("minecraft:")
                        && name.ends_with("_concrete_powder") =>
                    {
                        map_color()
                    }
                    name => sunrise_client::animate_tick::fixed_falling_dust_color(name)
                        .or_else(|| self.terrain_particle_tint(state, pos))
                        .unwrap_or_else(map_color),
                };
                ParticleOptions::block(ty, String::new(), tint)
            }
            ParticleType::BlockMarker => ParticleOptions::block(ty, sprite(), 0xFFFFFF),
            _ => return None,
        };
        Some(options.with_source(&block.name))
    }
    pub(in crate::game) fn craft_recipe(&self) -> Option<(String, ItemStack)> {
        let n = self.common.craft_size;
        let grid: Vec<Option<ItemStack>> = (0..n * n)
            .map(|i| self.common.craft_grid[i].clone())
            .collect();
        let scale = |id: i32| self.common.maps.data.get(&id).map(|m| m.scale);
        self.recipes()
            .find_with(&self.common.loop_context, &grid, n, &scale)
            .map(|(name, result)| (name.to_owned(), result))
    }
    pub(in crate::game) fn display_index(&self) -> &sunrise_sim::recipe_book::RecipeIndex {
        if self.common.is_logical_server() {
            &self.stations().recipe_index
        } else {
            self.common
                .recipe_tables
                .synced_index
                .as_ref()
                .unwrap_or(&self.stations().recipe_index)
        }
    }
    pub fn falling_block_face_sprite(&self, state: BlockStateId) -> Option<(String, u32)> {
        let pack = &self.client.client_pack;
        let models = pack
            .render
            .model_catalog
            .collect(state, sunrise_core::position::BlockPos::new(0, 0, 0))
            .ok()?;
        let face = models
            .iter()
            .flat_map(|(_, m)| m.faces.iter())
            .find(|f| f.quad.facing == sunrise_assets::model::Direction::Up)
            .or_else(|| models.iter().flat_map(|(_, m)| m.faces.iter()).next())?;
        let name = pack.layer_names.get(&face.layer)?.clone();
        let tint = if face.quad.tint_index >= 0 {
            let n = &self.common.res.facts().get(state).name;
            if n.contains("leaves") || n.contains("vine") {
                0x48B518
            } else {
                0x91BD59
            }
        } else {
            0xFFFFFF
        };
        Some((name, tint))
    }
    /// The biome colours used by the mesher and client tint queries. On a
    /// world without its own table, a colour refresh updates the pack's table.
    pub(in crate::game::client_side) fn mesh_biomes(&self) -> &Arc<Biomes> {
        self.common
            .world_biomes
            .as_ref()
            .unwrap_or(&self.client.client_pack.context.chunk.biomes)
    }
    /// The server's recipes a client was sent, over its own; None on a
    /// server and until a server sends any.
    pub(in crate::game) fn synced_recipes(&self) -> Option<&sunrise_sim::crafting::Recipes> {
        self.common.recipe_tables.synced.as_ref()
    }
    pub(in crate::game) fn client_recipes(&self) -> &Recipes {
        self.synced_recipes()
            .unwrap_or_else(|| self.common.res.recipes())
    }
    pub fn worker_context(&self) -> Arc<WorkerContext> {
        self.client
            .client_pack
            .context_with(self.mesh_biomes().clone())
    }

    pub(in crate::game) fn block_particle_sprite(
        client_pack: &ClientPackResources,
        state: BlockStateId,
    ) -> Option<String> {
        let catalog = &client_pack.render.model_catalog;
        let layer = catalog.particle(state).or_else(|| {
            let models = catalog
                .collect(state, sunrise_core::position::BlockPos::new(0, 0, 0))
                .ok()?;
            models
                .iter()
                .flat_map(|(_, m)| m.faces.iter())
                .next()
                .map(|f| f.layer)
        })?;
        client_pack.layer_names.get(&layer).cloned()
    }

    pub(in crate::game) fn particle_sprite(
        &self,
        state: BlockStateId,
        pos: [i32; 3],
    ) -> Option<(String, u32)> {
        let sprite = Self::particle_atlas_sprite(Self::block_particle_sprite(
            &self.client.client_pack,
            state,
        )?);
        let tint = self.terrain_particle_tint(state, pos).unwrap_or(0xFFFFFF);
        Some((sprite, tint))
    }

    pub(in crate::game) fn terrain_particle_tint(
        &self,
        state: BlockStateId,
        pos: [i32; 3],
    ) -> Option<u32> {
        let chunks = &self.common.world.chunks;
        let (cx, cz) = (pos[0] >> 4, pos[2] >> 4);
        let region: [[Option<&sunrise_level::chunk::Chunk>; 3]; 3] = std::array::from_fn(|dz| {
            std::array::from_fn(|dx| {
                chunks
                    .get(&(cx + dx as i32 - 1, cz + dz as i32 - 1))
                    .map(|c| &**c)
            })
        });
        self.client
            .client_pack
            .context
            .mesh
            .tables
            .terrain_particle_tint(
                state,
                pos,
                &region,
                world_biomes!(self),
                self.client.streaming.biome_zoom,
                self.client.streaming.biome_blend,
                self.client.streaming.nether_shading,
            )
    }
}
