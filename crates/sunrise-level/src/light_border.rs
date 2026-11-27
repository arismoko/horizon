//! The published light of columns the level does not hold.
//!
//! `LayerLightSectionStorage.updateSectionStatus` keeps sections touched by a non-air section
//! (the full 3x3x3 cube). `queueLightRemoval` drops a column's own support, but keeps its
//! last stored layers where another column still supports them.

use std::{borrow::Cow, sync::Arc};

use crate::chunk::{Chunk, Light, StoredLight, index};
use crate::light::{SectionStatus, remove_light_layers, update_light_layers};
use crate::section_mask::SectionMask;
use crate::world::World;

/// Retained or newly supported layers of a missing column, indexed from its lower pad.
#[derive(Default)]
pub struct Border(Vec<Layer>);

struct Layer {
    section: usize,
    sky: bool,
    block: bool,
    arrays: [bool; 2],
    values: Light,
}

fn neighbours((x, z): (i32, i32)) -> impl Iterator<Item = (i32, i32)> {
    (-1..=1).flat_map(move |dz| {
        (-1..=1).filter_map(move |dx| ((dx, dz) != (0, 0)).then_some((x + dx, z + dz)))
    })
}

impl World {
    fn border_support(&self, key: (i32, i32), except: Option<(i32, i32)>) -> SectionMask {
        neighbours(key).filter(|&near| except != Some(near)).fold(
            SectionMask::EMPTY,
            |support, near| {
                support
                    | self.chunks.resident_get(&near).map_or_else(
                        || {
                            self.proto_light_support
                                .get(&near)
                                .copied()
                                .unwrap_or_default()
                        },
                        |chunk| chunk.light_support,
                    )
            },
        )
    }

    /// `SkyLightSectionStorage.getLightValue`: an unstored section uses the bottom row of
    /// the nearest stored sky layer above it (15 if there isn't one). Block light does not inherit.
    pub(crate) fn border_light(&self, key: (i32, i32), x: i32, y: i32, z: i32) -> u8 {
        let Some(border) = self.border.get(&key) else {
            return 0xF0;
        };
        let section = i64::from(y >> 4) - i64::from(self.layout().min_y() >> 4) + 1;
        let start = border
            .0
            .partition_point(|layer| (layer.section as i64) < section);
        let layers = &border.0[start..];
        let cell = index(x, y, z);
        let block = layers.first().map_or(0, |layer| {
            if layer.block && layer.section as i64 == section {
                layer.values.get(cell) & 15
            } else {
                0
            }
        });
        let sky = layers.iter().find(|layer| layer.sky).map_or(0xF0, |layer| {
            let i = if layer.section as i64 == section {
                cell
            } else {
                cell & 255
            };
            layer.values.get(i) & 0xF0
        });
        sky | block
    }

    /// Install a column with its light section status already sampled at publication, not on reads.
    pub fn install_light_column(&mut self, chunk: Chunk) {
        self.install_light_column_initialized(chunk, SectionStatus::default());
    }

    pub fn install_light_column_initialized(&mut self, chunk: Chunk, status: SectionStatus) {
        let key = (chunk.x, chunk.z);
        let proto = self.proto_light_support.remove(&key).unwrap_or_default();
        let mut chunk = Arc::new(chunk);
        if chunk
            .light_layers
            .is_some_and(|light| light.saved_arrays.is_some())
        {
            let earlier = self.border_support(key, None) | proto;
            let support = chunk.light_support | earlier;
            restore_saved_layers(
                &mut chunk,
                support,
                Some(InitialSky {
                    earlier,
                    border: self.border.get(&key),
                    occupied: status.occupied,
                    fully_lit_from: status.fully_lit_from,
                }),
            );
        }
        let old = self.chunks.insert(key, chunk);
        self.publish_light_border(key, old.map_or(proto, |c| c.light_support));
    }

    /// Hold a proto's section status through temporary worker checkout and replace
    /// it atomically with the full column's status on install.
    pub fn set_proto_light_support(&mut self, key: (i32, i32), support: SectionMask) {
        let before = if support.is_empty() {
            self.proto_light_support.remove(&key)
        } else {
            self.proto_light_support.insert(key, support)
        }
        .unwrap_or_default();
        self.publish_light_support(key, before, support);
    }

    /// A newly published non-air section can initialize layers in an adjacent saved column.
    fn publish_saved_neighbour_support(&mut self, key: (i32, i32), added: SectionMask) {
        if added.is_empty() {
            return;
        }
        for (x, z) in neighbours(key) {
            let Some(chunk) = self.chunks.get_mut(&(x, z)) else {
                continue;
            };
            let (stored, changed) = restore_saved_layers(chunk, added, None);
            if !changed.is_empty() {
                self.light_updated(x, z, &changed);
            }
            if !stored.is_empty()
                && let Some(log) = &mut self.light_log
            {
                *log.entry((x, z)).or_default() |= stored;
            }
        }
    }

    /// A resident saved column may lose storage even though the column itself stays loaded.
    fn expire_saved_neighbour_support(&mut self, key: (i32, i32), lost: SectionMask) {
        if lost.is_empty() {
            return;
        }
        for near in neighbours(key) {
            let Some(chunk) = self.chunks.get(&near) else {
                continue;
            };
            let Some(StoredLight {
                masks: [sky, block],
                saved_arrays: Some(_),
            }) = chunk.light_layers
            else {
                continue;
            };
            let removed =
                lost & (sky | block) & !(chunk.light_support | self.border_support(near, None));
            if removed.is_empty() {
                continue;
            }
            let chunk = self.chunks.get_mut(&near).expect("resident saved column");
            let changed = remove_light_layers(Arc::make_mut(chunk), removed);
            self.modified.insert(near);
            if !changed.is_empty() {
                self.light_updated(near.0, near.1, &changed);
            }
        }
    }

    /// Update the neighbouring missing columns for a new section-status snapshot.
    pub fn publish_light_border(&mut self, key: (i32, i32), before: SectionMask) {
        self.border.remove(&key);
        self.publish_light_support(key, before, self.chunks[&key].light_support);
    }

    fn publish_light_support(
        &mut self,
        key: (i32, i32),
        before: SectionMask,
        current: SectionMask,
    ) {
        if current == before {
            return;
        }
        self.publish_saved_neighbour_support(key, current & !before);
        self.expire_saved_neighbour_support(key, before & !current);
        for near in neighbours(key).filter(|near| self.chunks.resident_get(near).is_none()) {
            let other = self.border_support(near, Some(key));
            let added = current & !before;
            let supported = current | other;
            if added.is_empty() && !self.border.contains_key(&near) {
                continue;
            }
            let sky = self.shape.sky_light;
            let border = self.border.entry(near).or_default();
            for section in added.iter() {
                let i = border.0.partition_point(|layer| layer.section < section);
                if border
                    .0
                    .get(i)
                    .is_some_and(|layer| layer.section == section)
                {
                    continue;
                }
                // `SkyLightSectionStorage.createDataLayer`: an unlit column starts dark, or
                // copies the bottom row of an existing sky layer above the newly stored one.
                let values = match border.0[i..]
                    .iter()
                    .find(|layer| layer.sky)
                    .map(|layer| &layer.values)
                {
                    None => Light::Uniform(0),
                    Some(Light::Uniform(value)) => Light::Uniform(value & 0xF0),
                    Some(Light::Block(_, sky)) => Light::Uniform(sky << 4),
                    Some(light) => {
                        let mut data = Box::new([0; 4096]);
                        for (i, v) in data.iter_mut().enumerate() {
                            *v = light.get(i & 255) & 0xF0;
                        }
                        Light::packed(data)
                    }
                };
                border.0.insert(
                    i,
                    Layer {
                        section,
                        sky,
                        block: true,
                        arrays: [false; 2],
                        values,
                    },
                );
            }
            // Section status adds immediately; pending removals are applied when light publishes.
            border.0.retain(|layer| supported.contains(layer.section));
            if border.0.is_empty() {
                self.border.remove(&near);
            }
        }
    }

    /// The column has just left: keep only its actually stored layers a neighbour still supports.
    pub(crate) fn retain_border(&mut self, key: (i32, i32), chunk: &Chunk) {
        self.expire_saved_neighbour_support(key, chunk.light_support);
        let supported = self.border_support(key, None);
        let kept: Vec<_> = supported
            .iter()
            .filter_map(|b| {
                let light = chunk.lit.then(|| chunk.light_section(b)).flatten()?;
                let [sky, block] = chunk
                    .stored_light_layers(b)
                    .unwrap_or([self.shape.sky_light, true]);
                (sky || block).then(|| Layer {
                    section: b,
                    sky,
                    block,
                    arrays: chunk
                        .stored_light()
                        .and_then(|m| m.saved_arrays(b))
                        .unwrap_or_default(),
                    values: light.light.clone(),
                })
            })
            .collect();
        if kept.is_empty() {
            self.border.remove(&key);
        } else {
            self.border.insert(key, Border(kept));
        }
        self.prune_border_neighbours(key);
    }

    fn prune_border_neighbours(&mut self, key: (i32, i32)) {
        for near in neighbours(key) {
            if !self.border.contains_key(&near) {
                continue;
            }
            let supported = self.border_support(near, None);
            let border = self.border.get_mut(&near).expect("border exists");
            border.0.retain(|layer| supported.contains(layer.section));
            if border.0.is_empty() {
                self.border.remove(&near);
            }
        }
    }
}

#[derive(Clone, Copy)]
struct InitialSky<'a> {
    earlier: SectionMask,
    border: Option<&'a Border>,
    occupied: SectionMask,
    fully_lit_from: Option<usize>,
}

/// Saved arrays were queued independently of section status. Initialize the omitted layers
/// against storage already present in this column, then apply `SkyLightEngine.setLightEnabled`.
/// Later status additions use the enabled column's live sky storage instead.
fn restore_saved_layers(
    chunk: &mut Arc<Chunk>,
    supported: SectionMask,
    initializing: Option<InitialSky<'_>>,
) -> (SectionMask, Vec<usize>) {
    let Some(StoredLight {
        masks: [sky, block],
        saved_arrays: Some(mut arrays),
    }) = chunk.light_layers
    else {
        return Default::default();
    };
    let new_sky = if chunk.light_sky {
        supported & !sky
    } else {
        SectionMask::EMPTY
    };
    let new_block = supported & !block;
    let added = new_sky | new_block;
    if added.is_empty() {
        return Default::default();
    }

    const DARK: &[u8] = &[0; 2048];
    const OPEN: &[u8] = &[0xFF; 2048];
    let border = initializing
        .and_then(|initial| initial.border)
        .map_or(&[][..], |border| &border.0);
    let mut layers = vec![(None, None); chunk.sections.len() + 2];
    for b in new_block.iter() {
        layers[b].1 = Some(
            if let Some(layer) = border.iter().find(|l| l.section == b && l.block) {
                if layer.arrays[1] {
                    arrays[1].insert(b);
                }
                if layer.values.any(15) {
                    Cow::Owned(
                        (0..2048)
                            .map(|i| {
                                (layer.values.get(2 * i) & 15) | (layer.values.get(2 * i + 1) << 4)
                            })
                            .collect(),
                    )
                } else {
                    Cow::Borrowed(DARK)
                }
            } else {
                Cow::Borrowed(DARK)
            },
        );
    }
    let initial_sources = initializing.filter(|_| !new_sky.is_empty()).map(|initial| {
        let mut source = vec![None; layers.len()];
        let mut active = initial.earlier;
        for b in active.iter() {
            if sky.contains(b) || border.iter().any(|l| l.section == b && l.sky) {
                source[b] = Some(b);
            }
        }
        // `updateSectionStatus` visits non-air sections from the bottom up. It
        // initializes the section itself before its neighbours, so its queued
        // array can seed the newly allocated layer underneath.
        for s in initial.occupied.iter() {
            for b in [s + 1, s, s + 2] {
                if !active.contains(b) {
                    source[b] = if sky.contains(b) {
                        Some(b)
                    } else {
                        active
                            .from(b + 1)
                            .iter()
                            .next()
                            .and_then(|above| source[above])
                    };
                    active.insert(b);
                }
            }
        }
        source
    });
    for b in new_sky.iter() {
        let source_section = initial_sources
            .as_ref()
            .map_or_else(|| sky.from(b + 1).iter().next(), |sources| sources[b]);
        let source = source_section.and_then(|s| {
            if sky.contains(s) {
                Some((
                    s,
                    &chunk
                        .light_section(s)
                        .expect("stored sky has a section")
                        .light,
                    arrays[0].contains(s),
                ))
            } else {
                border
                    .iter()
                    .find(|l| l.section == s && l.sky)
                    .map(|l| (s, &l.values, l.arrays[0]))
            }
        });
        let mut layer = Cow::Borrowed(if initializing.is_none() { OPEN } else { DARK });
        if let Some((s, light, materialized)) = source {
            if materialized {
                arrays[0].insert(b);
            }
            let sky_byte = |i| (light.get(2 * i) >> 4) | (light.get(2 * i + 1) & 0xF0);
            layer = match light {
                Light::Block(_, 0) => Cow::Borrowed(DARK),
                Light::Uniform(v) if v & 0xF0 == 0 => Cow::Borrowed(DARK),
                Light::Uniform(v) if v & 0xF0 == 0xF0 => Cow::Borrowed(OPEN),
                _ if s == b => Cow::Owned((0..2048).map(sky_byte).collect()),
                _ => Cow::Owned(std::array::from_fn::<_, 128, _>(sky_byte).repeat(16)),
            };
        }
        if initializing
            .and_then(|initial| initial.fully_lit_from)
            .is_some_and(|start| b >= start)
            && !arrays[0].contains(b)
            && layer.as_ref() == DARK
        {
            layer = Cow::Borrowed(OPEN);
        }
        layers[b].0 = Some(layer);
    }
    let sky_light = chunk.light_sky;
    let chunk = Arc::make_mut(chunk);
    let changed = update_light_layers(chunk, &layers, sky_light);
    chunk
        .light_layers
        .as_mut()
        .expect("light update stored layers")
        .saved_arrays = Some(arrays);
    (added, changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{chunk::SECTIONS, light::light_from_layers};
    use sunrise_world::registry::BlockStateId;

    #[test]
    fn saved_padding_uses_initialization_and_already_stored_sky() {
        // Same fixture as the saved-padding Java oracle: a proto has opened the
        // target's lowest build section, but not its lower light pad.
        use crate::{
            chunk::MIN_Y,
            light::{light_from_saved_layers, tests::tables},
        };
        let top = MIN_Y + (SECTIONS as i32 - 1) * 16;
        let mut chunk = Chunk::empty(0, 0);
        chunk.set(0, MIN_Y, 0, 1);
        chunk.set(0, top, 0, 1);
        let mut layers = vec![(None, None); SECTIONS + 2];
        let sky: Vec<_> = (0..2048)
            .map(|i| {
                let v = (((i * 2) & 15) + ((i >> 3) & 15) * 2 + (i >> 7) * 3 + 7) as u8;
                (v & 15) | (v.wrapping_add(1) << 4)
            })
            .collect();
        layers[1] = (Some(sky), Some(vec![0; 2048]));
        light_from_saved_layers(&mut chunk, &layers, true);
        let mut world = World::new(BlockStateId(0), 0);
        let status = tables().publish_section_status(&mut chunk, world.shape);
        let mut without_proto = World::new(BlockStateId(0), 0);
        without_proto.install_light_column_initialized(chunk.clone(), status);
        assert_eq!(without_proto.light(0, MIN_Y - 1, 0), 0x70);
        assert_eq!(
            without_proto.chunks[&(0, 0)].stored_light_layers(3),
            Some([false; 2])
        );
        // Announcing the same proto later still publishes the missing layer, even
        // when the values did not change from the old read cache.
        without_proto.light_log = Some(Default::default());
        without_proto.set_proto_light_support((1, 0), SectionMask::from_iter([1, 2, 3]));
        assert_eq!(
            without_proto.chunks[&(0, 0)].stored_light_layers(3),
            Some([true; 2])
        );
        assert_eq!(
            without_proto.light_log.as_ref().unwrap()[&(0, 0)],
            SectionMask::single(3)
        );

        world.set_proto_light_support((1, 0), SectionMask::from_iter([1, 2, 3]));
        world.install_light_column_initialized(chunk, status);
        let stored = world.chunks[&(0, 0)].stored_light().unwrap();
        assert_eq!(stored.presence(3), [true; 2]);
        assert_eq!(stored.saved_arrays(0), Some([true, false]));
        assert_eq!(stored.saved_arrays(1), Some([true, true]));
        assert_eq!(world.light(0, top + 16, 0), 0xF0);
        assert_eq!(world.light(0, MIN_Y - 1, 0), 0x70);
        assert_eq!(world.light(1, MIN_Y - 16, 1), 0xA0);
    }

    #[test]
    fn saved_void_pad_is_recreated_after_full_or_proto_support_returns() {
        use crate::{
            chunk::MIN_Y,
            light::{light_from_saved_layers, tests::tables},
        };
        for proto in [false, true] {
            let mut world = World::new(BlockStateId(0), 0);
            let mut chunk = Chunk::empty(0, 0);
            light_from_saved_layers::<&[u8]>(&mut chunk, &[], true);
            let status = tables().publish_section_status(&mut chunk, world.shape);
            let support = |world: &mut World| {
                if proto {
                    world.set_proto_light_support((1, 0), SectionMask::below(3));
                } else {
                    let mut neighbour = Chunk::empty(1, 0);
                    neighbour.add_light_support(0);
                    world.install_light_column(neighbour);
                }
            };
            support(&mut world);
            world.install_light_column_initialized(chunk, status);
            assert_eq!(
                world.chunks[&(0, 0)].stored_light_layers(0),
                Some([true; 2])
            );
            assert_eq!(world.light(0, MIN_Y - 1, 0), 0);

            if proto {
                world.set_proto_light_support((1, 0), SectionMask::EMPTY);
            } else {
                world.unload_chunk((1, 0)).unwrap();
            }
            assert_eq!(
                world.chunks[&(0, 0)].stored_light_layers(0),
                Some([false; 2])
            );
            assert_eq!(world.light(0, MIN_Y - 1, 0), 0xF0);

            // The void column remained enabled while the support was away.
            support(&mut world);
            assert_eq!(
                world.chunks[&(0, 0)].stored_light_layers(0),
                Some([true; 2])
            );
            assert_eq!(world.light(0, MIN_Y - 1, 0), 0xF0);
        }
    }

    #[test]
    fn a_retained_gap_uses_the_nearest_stored_sky_not_cached_inheritance() {
        let mut world = World::new(BlockStateId(0), 0);
        let mut neighbour = Chunk::empty(0, 0);
        neighbour.add_light_support(1);
        neighbour.add_light_support(5);
        world.install_light_column(neighbour);
        let mut chunk = Chunk::empty(1, 0);
        chunk.add_light_support(3);
        let mut layers = vec![(None, None); SECTIONS + 2];
        layers[2].1 = Some(vec![0x77; 2048]);
        layers[3].0 = Some(vec![0; 2048]);
        layers[4].0 = Some(vec![0xAA; 2048]); // supported only by the column being removed
        layers[6].0 = Some(vec![0xCC; 2048]);
        light_from_layers(&mut chunk, &layers, true);
        world.install_light_column(chunk);
        world.unload_chunk((1, 0)).unwrap();
        assert_eq!(world.light(16, -48, 0), 7); // block-only section borrows from the dark row
        assert_eq!(world.light(16, -16, 0), 0xC0); // removed row borrows from the higher row
        assert_eq!(world.light(16, -200, 0), 0); // no fabricated lower pad or stale packed sky
        assert_eq!(world.light(16, 48, 0), 0xF0);
    }
}
