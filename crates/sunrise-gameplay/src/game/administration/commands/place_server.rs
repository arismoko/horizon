use super::{R, err};
use crate::game::commands::place::read_template_data;
use sunrise_core::nbt::Tag;

impl crate::game::ServerGame<'_> {
    /// `PlaceCommand.checkLoaded`: every chunk of the inclusive range.
    pub(in crate::game::server_side) fn place_check_loaded(
        &self,
        min: [i32; 2],
        max: [i32; 2],
    ) -> R<()> {
        let dimension = self
            .server
            .command_source
            .as_ref()
            .map_or(self.common.dim, |s| s.dimension);
        for cx in min[0]..=max[0] {
            for cz in min[1]..=max[1] {
                if dimension != self.common.dim || !self.common.world.chunks.contains_key(&(cx, cz))
                {
                    return Err(err("argument.pos.unloaded", &[]));
                }
            }
        }
        Ok(())
    }
    /// A saved world template, or the winning passive/world-pack resource.
    pub fn read_template_tag(&self, id: &str) -> Option<Tag> {
        use sunrise_assets::resource_source::{Resource, stays_inside};
        let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
        if !stays_inside(ns) || !stays_inside(path) {
            return None;
        }
        let saved = self
            .server
            .world_root
            .join("generated")
            .join(ns)
            .join("structures")
            .join(format!("{path}.nbt"));
        let prepared = self
            .common
            .options
            .resources
            .join("worldgen/structure")
            .join(ns)
            .join(format!("{path}.nbt"));
        let name = format!("data/{ns}/structure/{path}.nbt");
        let resource = self.server.fns.0.data_resource(&name).or_else(|| {
            self.common
                .pack
                .data_source
                .contains(&name)
                .then(|| Resource::file(self.common.pack.data_source.clone(), name))
        });
        read_template_data(&saved, &prepared, resource)
    }
}
