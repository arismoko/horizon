//! A server mod's bulk block edit (`edit_blocks`): its palette and
//! filters compile to the command matcher, its heights read, and its runs made
//! at once through `World::edit_runs`, with what `fill ... strict` does for
//! the block entities and the watched blocks of each block changed.
use crate::game::ServerGame;
use mod_api::Heightmap;
use mod_api::{BlockEdit, BlockEditError};
use std::sync::Arc;
use sunrise_gameplay_common::game::block_edits_common::Tracked;
#[allow(unused_imports)]
pub(in crate::game) use sunrise_gameplay_common::game::block_edits_common::column_height;
use sunrise_gameplay_common::game::transactions::{
    ReplayEntity, StrictBlockStep, StrictBlockSteps,
};
use sunrise_level::{
    block_edit::{BulkPublication, Run},
    containers::BlockEntity,
    fast_hash::{FxHashMap, FxHashSet},
    world::World,
};
use sunrise_world::registry::BlockStateId;

impl ServerGame<'_> {
    /// Make a mod's block edit; how many blocks changed, or why none did.
    /// Every palette entry, filter and run is checked before anything
    /// changes: an invalid typed state or predicate (NBT cannot be represented),
    /// a run naming a palette entry
    /// or filter past the end, or more than 65536 filters, and nothing is
    /// made.
    pub fn apply_block_edit(&mut self, edit: &BlockEdit) -> Result<u64, BlockEditError> {
        let sunrise_gameplay_common::game::api_values::block_edit::PreparedBlockEdit {
            palette,
            filters,
        } = sunrise_gameplay_common::game::api_values::block_edit::prepare(edit)?;
        let runs = resolve_runs(edit, &palette, filters.len(), |h, x, z| {
            self.common.edit_height(h, x, z)
        })?;
        // (what the registry, the tags and the facts tell of a state, beside the world being changed)
        self.ensure_stateful_entity_table();
        let stateful = self.common.world.block_entities.stateful_entities();
        let registry = self.common.pack.assets.registry;
        // (the tags as `block_matches` reads them for commands)
        let world_items = match &self.server.world_tables.0 {
            Some(t) => &t.items,
            None => self.common.res.items(),
        };
        let tags = [&self.common.res.items().block_tags, &world_items.block_tags];
        let facts = self.common.res.facts();
        let watched = std::mem::take(&mut self.common.world.watched);
        let matches = |f: usize, s| filters[f].matches(registry, tags, BlockStateId(s));
        let track = |s| {
            let f = facts.get(BlockStateId(s));
            watched.get(s as usize).copied().unwrap_or(false)
                || f.block_entity
                || stateful.get(s as usize).copied().unwrap_or(false)
                || BlockEntity::for_block(&f.name).is_some()
        };
        let mut steps = StrictBlockSteps::new();
        let (out, publication) =
            if let Some(frame) = self.common.transactions.level_frame(self.common.dim) {
                let (out, publication) = self.common.world.stage_edit_runs(
                    &runs,
                    matches,
                    track,
                    |pos, old, new, clears_group, old_group| {
                        frame.keep_bulk_block(pos, old);
                        if old_group.is_some() {
                            frame.keep_connection_group(pos, old_group);
                        }
                        let to = BlockStateId(new);
                        steps
                            .entry(pos)
                            .and_modify(|step| {
                                step.to = to;
                                step.clear_connection_group |= clears_group;
                            })
                            .or_insert(StrictBlockStep {
                                to,
                                row: None,
                                created_generation: None,
                                clear_connection_group: clears_group,
                            });
                    },
                );
                (out, Some(publication))
            } else {
                (self.common.world.edit_runs(&runs, matches, track), None)
            };
        self.common.world.watched = watched;
        // The block entities the new blocks bring, as `native_set_block` makes them (a mod block's own,
        // included), in the order `replace_block_entities` takes them: the positions needing a fresh entity.
        let resources = Arc::clone(&self.common.res);
        let facts = resources.facts();
        let name = |s: u32| facts.get(BlockStateId(s)).name.clone();
        let wanted: Vec<BlockStateId> = out
            .tracked
            .iter()
            .filter(|&&((x, y, z), old, new)| {
                name(old) != name(new)
                    || (!self.common.world.block_entities.is_taken(&(x, y, z))
                        && !self
                            .common
                            .world
                            .block_entities
                            .get(&(x, y, z))
                            .is_some_and(|e| e.fits(&name(new))))
            })
            .map(|&(_, _, new)| BlockStateId(new))
            .collect();
        let mut made =
            make_block_entities(&wanted, |state| self.new_block_entity_for_state(*state))
                .into_iter();
        if publication.is_some() {
            // Keep the original row, including absence, before the strict writer creates or
            // removes a resident. Keep its drops suppressed just as in an immediate bulk edit.
            for &(pos, old, new) in &out.tracked {
                if name(old) != name(new)
                    || (!self.common.world.block_entities.is_taken(&pos)
                        && !self
                            .common
                            .world
                            .block_entities
                            .get(&pos)
                            .is_some_and(|row| row.fits(&name(new))))
                {
                    self.journal_block_entity(pos);
                }
            }
        }
        let removed = replace_block_entities(
            &mut self.common.world,
            &out.tracked,
            |s| facts.get(BlockStateId(s)).name.clone(),
            |_| made.next().flatten(),
            publication.as_ref().map(|_| &mut steps),
        );
        if let Some(publication) = publication {
            self.after_commit(
                sunrise_gameplay_common::game::transactions::After::BulkBlocks(
                    publication,
                    out.tracked,
                    removed,
                    steps,
                ),
            );
        } else {
            for pos in removed {
                self.jukebox_removed(pos);
            }
        }
        Ok(out.changed)
    }

    pub(in crate::game::server_side) fn publish_bulk_edit(
        &mut self,
        publication: BulkPublication,
        tracked: Vec<Tracked>,
        removed: Vec<[i32; 3]>,
    ) {
        self.common.world.publish_bulk(publication);
        for (pos, old, new) in tracked {
            publish_tracked(&mut self.common.world, pos, old, new);
        }
        for pos in removed {
            self.jukebox_removed(pos);
        }
    }
}

/// `edit`'s runs as `World::edit_runs` takes them: each palette entry `block`
/// as its state in `palette`, each `y` counted from `height(surface, x, z)`
/// when the run names a heightmap (read before any change, once a column).
/// Err, with nothing resolved, for a run naming a palette entry or a filter
/// past the end, or for more filters than a run can name (65536).
fn resolve_runs(
    edit: &BlockEdit,
    palette: &[u32],
    filters: usize,
    mut height: impl FnMut(Heightmap, i32, i32) -> i32,
) -> Result<Vec<Run>, BlockEditError> {
    let mut heights: FxHashMap<(i32, i32, u8), i32> = FxHashMap::default();
    let mut runs = Vec::with_capacity(edit.runs.len());
    for r in &edit.runs {
        let Some(&state) = usize::try_from(r.block).ok().and_then(|b| palette.get(b)) else {
            return Err(BlockEditError::PaletteOutOfRange);
        };
        let only = match r.only {
            None => None,
            Some(f) if (f as usize) < filters => Some(f as usize),
            Some(_) => return Err(BlockEditError::FilterOutOfRange),
        };
        if r.height == 0 {
            continue;
        }
        let base = match r.surface {
            None => i64::from(r.y),
            Some(h) => {
                let top = *heights
                    .entry((r.x, r.z, h as u8))
                    .or_insert_with(|| height(h, r.x, r.z));
                i64::from(top) + i64::from(r.y)
            }
        };
        let clamp = |v: i64| v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
        runs.push(Run {
            x: r.x,
            z: r.z,
            y0: clamp(base),
            y1: clamp(base + i64::from(r.height) - 1),
            state,
            only,
        });
    }
    Ok(runs)
}

/// What `fill ... strict` does for the block entities and the watched blocks
/// of `tracked`'s changes: the watched writes recorded, the block entities
/// told of their new state, and where a different block replaced the block,
/// its block entity removed (as `place_state` does) and `new_entity(name)`'s,
/// if any, put in its place. Returns the positions whose block was replaced
/// (a jukebox there stops).
/// A block entity for each new block, in order: made for each position, as vanilla calls
/// `EntityBlock.newBlockEntity` for each (a mod's may number its instances). A block that makes none (stone) is
/// asked once.
fn make_block_entities<S: Eq + std::hash::Hash>(
    states: &[S],
    mut make: impl FnMut(&S) -> Option<BlockEntity>,
) -> Vec<Option<BlockEntity>> {
    let mut none: FxHashSet<&S> = FxHashSet::default();
    states
        .iter()
        .map(|n| {
            if none.contains(n) {
                return None;
            }
            let entity = make(n);
            if entity.is_none() {
                none.insert(n);
            }
            entity
        })
        .collect()
}

fn publish_tracked(world: &mut World, pos: (i32, i32, i32), old: u32, new: u32) {
    let watches = |id: u32| world.watched.get(id as usize).copied().unwrap_or(false);
    if watches(old) || watches(new) {
        world.watched_writes.push((pos, old, new));
    }
    world.block_entities.state_changed(pos, old, new);
}

fn replace_block_entities(
    world: &mut World,
    tracked: &[Tracked],
    name: impl Fn(u32) -> Arc<str>,
    mut new_entity: impl FnMut(&str) -> Option<BlockEntity>,
    mut staged: Option<&mut StrictBlockSteps>,
) -> Vec<[i32; 3]> {
    let mut replaced = Vec::new();
    for &((x, y, z), old, new) in tracked {
        if staged.is_none() {
            publish_tracked(world, (x, y, z), old, new);
        }
        let (was, now) = (name(old), name(new));
        if was != now {
            replaced.push([x, y, z]);
        }
        // LevelChunk.setBlockState: the entity at the position keeps its
        // place only while it is valid for the block now there
        // (`BlockEntity.isValidBlockState`); a mismatched or missing one
        // makes way for the block's own.
        if was != now
            || (!world.block_entities.is_taken(&(x, y, z))
                && !world
                    .block_entities
                    .get(&(x, y, z))
                    .is_some_and(|e| e.fits(&now)))
        {
            let pos = (x, y, z);
            let mut step = staged.as_deref_mut().and_then(|steps| steps.get_mut(&pos));
            match step.as_deref_mut() {
                Some(step) if step.row.is_none() => {
                    step.row = Some(ReplayEntity::take(world, pos));
                }
                _ => {
                    world.block_entities.remove(&pos);
                }
            }
            if step.is_some() {
                world.block_names.remove(&pos);
                world.forget_block_entity_source_unmodified(pos);
            }
            let created_generation = new_entity(&now).and_then(|entity| {
                world.block_entities.insert(pos, entity);
                world.block_entities.generation(&pos)
            });
            if let Some(step) = step {
                step.created_generation = created_generation;
            }
        }
    }
    replaced
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::native_hooks::SharedGameExt1 as _;

#[cfg(test)]
mod tests {
    use sunrise_gameplay_common::game::server_side::block_edits::tests::columns_world;

    use sunrise_gameplay_common::game::server_side::block_edits::tests::edit;

    use sunrise_gameplay_common::game::server_side::block_edits::tests::{height, names};

    use sunrise_gameplay_common::game::server_side::block_edits::tests::run;

    use sunrise_gameplay_common::game::server_side::block_edits::tests::world;

    use super::*;
    #[cfg(test)]
    use mod_api::BlockRun;
    use sunrise_gameplay_common::game::block_edits_common::{column_span, fill_column_blocks};
    use sunrise_level::chunk::{Chunk, LevelShape};

    #[test]
    #[ignore = "daily"]
    fn aborted_bulk_replacement_restores_loaned_block_entity_lease() {
        use crate::game::native_entity_refs::tests::with_game;
        use mod_api::BlockUpdateFlags;
        use sunrise_gameplay_common::game::api_values::TryIntoApi as _;
        use sunrise_gameplay_common::game::native_block_entity_read::NativeBlockEntityReadGameExt1 as _;
        use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
        use sunrise_items::items::ItemStack;

        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level(game.dim.id()).unwrap();
            let state = |name| game.pack.assets.registry.default_state(name).unwrap();
            let (chest, stone, hopper) = (
                state("minecraft:chest"),
                state("minecraft:stone"),
                state("minecraft:hopper"),
            );
            let to_stone = [1, 64, 0];
            let to_hopper = [2, 64, 0];
            let diamond = ItemStack::new("minecraft:diamond", 1);
            for pos in [to_stone, to_hopper] {
                assert_eq!(
                    level.set_block(
                        game,
                        pos,
                        chest.try_into_api().unwrap(),
                        BlockUpdateFlags::NONE
                    ),
                    Ok(true)
                );
                game.world
                    .block_entities
                    .get_mut(&(pos[0], pos[1], pos[2]))
                    .unwrap()
                    .slots_mut()[0] = Some(diamond.clone());
            }
            let h1 = game.block_entity(level, to_stone).unwrap();
            let h2 = game.block_entity(level, to_hopper).unwrap();
            let (lease1, entity1) = game.world.block_entities.take(&(1, 64, 0)).unwrap();
            let (lease2, entity2) = game.world.block_entities.take(&(2, 64, 0)).unwrap();

            let edit = BlockEdit {
                palette: vec![
                    stone.try_into_api().unwrap(),
                    hopper.try_into_api().unwrap(),
                ],
                filters: vec![],
                runs: [(to_stone, 0), (to_hopper, 1)]
                    .into_iter()
                    .map(|([x, y, z], block)| BlockRun {
                        x,
                        y,
                        z,
                        block,
                        height: 1,
                        surface: None,
                        only: None,
                    })
                    .collect(),
            };

            game.storage_scope(|game, _| {
                game.storage_scope(|game, _| {
                    assert_eq!(level.edit_blocks(game, &edit), Ok(2));
                    ((), true)
                });
                assert!(game.block_entity(level, to_stone).is_none());
                assert_ne!(game.block_entity(level, to_hopper), Some(h2));
                assert!(game.block_entity(level, to_hopper).is_some());
                ((), false)
            });

            assert_eq!(game.world.block(1, 64, 0), chest);
            assert_eq!(game.world.block(2, 64, 0), chest);
            assert!(game.world.block_entities.put(lease1, entity1));
            assert!(game.world.block_entities.put(lease2, entity2));
            assert_eq!(game.block_entity(level, to_stone), Some(h1));
            assert_eq!(game.block_entity(level, to_hopper), Some(h2));
            for pos in [(1, 64, 0), (2, 64, 0)] {
                assert_eq!(
                    game.world.block_entities.get(&pos).unwrap().slots()[0],
                    Some(diamond.clone())
                );
            }

            let (lease, entity) = game.world.block_entities.take(&(1, 64, 0)).unwrap();
            game.storage_scope(|game, _| {
                assert_eq!(level.edit_blocks(game, &edit), Ok(2));
                ((), true)
            });
            assert!(!game.world.block_entities.put(lease, entity));
            assert_eq!(game.world.block(1, 64, 0), stone);
            assert!(game.world.block_entities.get(&(1, 64, 0)).is_none());
        });
    }

    #[test]
    #[ignore = "daily"]
    fn mixed_bulk_and_ordinary_writes_publish_only_on_outer_commit() {
        use crate::game::native_entity_refs::tests::with_game;
        use mod_api::BlockUpdateFlags;
        use sunrise_gameplay_common::game::api_values::TryIntoApi as _;
        use sunrise_gameplay_common::game::native_block_entity_read::NativeBlockEntityReadGameExt1 as _;
        use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.world.block_log = Some(Vec::new());
            let level = game.level(game.dim.id()).unwrap();
            let state = |name| game.pack.assets.registry.default_state(name).unwrap();
            let (air, stone, chest, dirt) = (
                state("minecraft:air"),
                state("minecraft:stone"),
                state("minecraft:chest"),
                state("minecraft:dirt"),
            );
            let original = [1, 64, 0];
            let new_holder = [2, 64, 0];
            let ordinary = [3, 64, 0];
            let sync = [4, 64, 0];
            for pos in [original, sync] {
                assert_eq!(
                    level.set_block(
                        game,
                        pos,
                        chest.try_into_api().unwrap(),
                        BlockUpdateFlags::NONE
                    ),
                    Ok(true)
                );
            }
            let holder = game.block_entity(level, original).unwrap();
            let explicit = game.block_entity(level, sync).unwrap();
            let edit = BlockEdit {
                palette: vec![stone.try_into_api().unwrap(), chest.try_into_api().unwrap()],
                filters: vec![],
                runs: [(original, 0), (new_holder, 1)]
                    .into_iter()
                    .map(|([x, y, z], block)| BlockRun {
                        x,
                        y,
                        z,
                        block,
                        height: 1,
                        surface: None,
                        only: None,
                    })
                    .collect(),
            };
            game.world.modified.clear();
            game.world.dirty_light.clear();
            game.world.block_log.as_mut().unwrap().clear();

            for commit in [false, true] {
                game.storage_scope(|game, _| {
                    game.storage_scope(|game, _| {
                        assert_eq!(level.edit_blocks(game, &edit), Ok(2));
                        ((), true)
                    });
                    assert!(game.block_entity(level, original).is_none());
                    assert!(game.block_entity(level, new_holder).is_some());
                    for pos in [ordinary, original] {
                        assert_eq!(
                            level.set_block(
                                game,
                                pos,
                                dirt.try_into_api().unwrap(),
                                BlockUpdateFlags::CLIENTS
                            ),
                            Ok(true)
                        );
                    }
                    assert!(game.world.bulk_log.is_empty());
                    assert!(game.world.dirty_light.is_empty());
                    assert!(game.world.modified.is_empty());
                    assert!(game.world.block_log.as_ref().unwrap().is_empty());
                    ((), commit)
                });
                assert_eq!(
                    game.world.block(1, 64, 0),
                    if commit { dirt } else { chest }
                );
                assert_eq!(game.world.block(2, 64, 0), if commit { chest } else { air });
                assert_eq!(game.world.block(3, 64, 0), if commit { dirt } else { air });
                assert_eq!(
                    game.block_entity(level, original),
                    (!commit).then_some(holder)
                );
                assert_eq!(game.block_entity(level, new_holder).is_some(), commit);
                assert_eq!(game.world.modified.contains(&(0, 0)), commit);
                assert_eq!(game.world.dirty_light.contains(&(0, 0)), commit);
                assert_eq!(game.world.bulk_log.contains(&(0, 0)), commit);
            }
            assert_eq!(explicit.sync(game), Ok(()));
            assert!(game.world.block_log.as_ref().unwrap().contains(&(4, 64, 0)));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn swapped_dimension_bulk_edits_use_only_their_own_level_journal() {
        use crate::game::native_entity_refs::tests::with_game;
        use mod_api::BlockUpdateFlags;
        use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
        use sunrise_gameplay_common::game::{Dim, api_values::TryIntoApi as _};

        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.world.block_log = Some(Vec::new());
            let state = |name| game.pack.assets.registry.default_state(name).unwrap();
            let (stone, dirt, cobblestone) = (
                state("minecraft:stone"),
                state("minecraft:dirt"),
                state("minecraft:cobblestone"),
            );
            let overworld = game.level("minecraft:overworld").unwrap();
            assert_eq!(
                overworld.set_block(
                    game,
                    [1, 64, 0],
                    stone.try_into_api().unwrap(),
                    BlockUpdateFlags::NONE
                ),
                Ok(true)
            );
            let back = game.enter_command_level(Dim::Nether).unwrap();
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.world.block_log = Some(Vec::new());
            let nether = game.level("minecraft:the_nether").unwrap();
            assert_eq!(
                nether.set_block(
                    game,
                    [1, 64, 0],
                    dirt.try_into_api().unwrap(),
                    BlockUpdateFlags::NONE
                ),
                Ok(true)
            );
            game.world.modified.clear();
            game.world.dirty_light.clear();
            game.world.bulk_log.clear();
            game.enter_command_level(back).unwrap();
            game.world.modified.clear();
            game.world.dirty_light.clear();
            game.world.bulk_log.clear();

            let edit = BlockEdit {
                palette: vec![cobblestone.try_into_api().unwrap()],
                filters: vec![],
                runs: vec![BlockRun {
                    x: 1,
                    y: 64,
                    z: 0,
                    block: 0,
                    height: 1,
                    surface: None,
                    only: None,
                }],
            };

            game.storage_scope(|game, _| {
                // Only the overworld transaction is open. The parked nether commits immediately.
                assert_eq!(nether.edit_blocks(game, &edit), Ok(1));
                let back = game.enter_command_level(Dim::Nether).unwrap();
                assert_eq!(game.world.block(1, 64, 0), cobblestone);
                assert!(game.world.modified.contains(&(0, 0)));
                assert!(game.world.bulk_log.contains(&(0, 0)));
                // The parked overworld still has its own open frame.
                assert_eq!(overworld.edit_blocks(game, &edit), Ok(1));
                let parked = game.server.levels.0.parked_world(Dim::Overworld).unwrap();
                assert_eq!(parked.block(1, 64, 0), cobblestone);
                assert!(parked.modified.is_empty());
                assert!(parked.bulk_log.is_empty());

                // A nether transaction commits its own edits even with an overworld parent.
                game.storage_scope(|game, _| {
                    let reset = BlockEdit {
                        palette: vec![dirt.try_into_api().unwrap()],
                        ..edit
                    };
                    assert_eq!(nether.edit_blocks(game, &reset), Ok(1));
                    ((), true)
                });
                game.enter_command_level(back).unwrap();
                ((), false)
            });
            assert_eq!(game.world.block(1, 64, 0), stone);
            assert!(game.world.modified.is_empty());
            assert!(game.world.bulk_log.is_empty());
            assert_eq!(
                game.server
                    .levels
                    .0
                    .parked_world(Dim::Nether)
                    .unwrap()
                    .block(1, 64, 0),
                dirt
            );
        });
    }

    #[test]
    fn runs_naming_entries_past_the_end_are_refused_whole() {
        let palette = [1, 0];
        let ok = resolve_runs(&edit(vec![run(1, Some(0))]), &palette, 1, |_, _, _| 0).unwrap();
        assert_eq!(
            ok,
            [Run {
                x: 1,
                z: 2,
                y0: 3,
                y1: 6,
                state: 0,
                only: Some(0),
            }]
        );
        // a bad run after a good one: nothing at all, and no height read
        let mut asked = 0;
        let mut bad = |runs| {
            resolve_runs(&edit(runs), &palette, 1, |_, _, _| {
                asked += 1;
                0
            })
        };
        assert_eq!(
            bad(vec![run(0, None), run(2, None)]),
            Err(BlockEditError::PaletteOutOfRange)
        );
        assert_eq!(
            bad(vec![run(0, None), run(u32::MAX, None)]),
            Err(BlockEditError::PaletteOutOfRange)
        );
        assert_eq!(
            bad(vec![run(0, Some(1))]),
            Err(BlockEditError::FilterOutOfRange)
        );
        assert_eq!(
            bad(vec![run(0, Some(u32::MAX))]),
            Err(BlockEditError::FilterOutOfRange)
        );
        assert_eq!(asked, 0);
    }

    #[test]
    fn a_run_counts_from_its_heightmap_read_once_a_column() {
        let mut r = run(0, None);
        r.surface = Some(Heightmap::OceanFloor);
        r.y = -2;
        let mut asked = Vec::new();
        let runs = resolve_runs(&edit(vec![r, r]), &[7], 0, |h, x, z| {
            asked.push((h, x, z));
            70
        })
        .unwrap();
        assert_eq!(asked, [(Heightmap::OceanFloor, 1, 2)]);
        assert_eq!((runs[0].y0, runs[0].y1, runs[0].state), (68, 71, 7));
        assert_eq!(runs[1], runs[0]);
        // a run far past the world's height clamps rather than wraps
        let mut high = run(0, None);
        high.y = i32::MAX;
        high.height = u32::MAX;
        let runs = resolve_runs(&edit(vec![high]), &[7], 0, |_, _, _| 0).unwrap();
        assert_eq!((runs[0].y0, runs[0].y1), (i32::MAX, i32::MAX));
    }

    #[test]
    fn a_chest_replaced_by_stone_loses_its_block_entity() {
        let mut w = world();
        w.set(1, 5, 1, BlockStateId(2)).unwrap();
        let chest = BlockEntity::for_block("minecraft:chest").expect("a chest's");
        w.block_entities.insert((1, 5, 1), chest);
        let out = w.edit_runs(
            &[Run {
                x: 1,
                z: 1,
                y0: 4,
                y1: 6,
                state: 1,
                only: None,
            }],
            |_, _| true,
            |s| s == 2,
        );
        assert_eq!(out.tracked, [((1, 5, 1), 2, 1)]);
        let replaced =
            replace_block_entities(&mut w, &out.tracked, names, BlockEntity::for_block, None);
        assert_eq!(replaced, [[1, 5, 1]]);
        assert!(w.block_entities.get(&(1, 5, 1)).is_none());
        assert_eq!(w.block(1, 5, 1).0, 1);
    }

    #[test]
    fn a_block_entitys_published_shapes_go_with_it_when_its_block_changes() {
        let mut w = world();
        w.set(1, 5, 1, BlockStateId(3)).unwrap();
        let mut press = sunrise_level::containers::ModBlockEntity::new(
            "machines:press".into(),
            Default::default(),
        );
        press.set_shapes(Some(Arc::new(
            sunrise_level::containers::PublishedShapes::new(mod_api::BlockEntityShapes {
                outline: vec![],
                collision: vec![[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]],
            })
            .unwrap(),
        )));
        w.block_entities
            .insert((1, 5, 1), BlockEntity::Mod(Box::new(press)));
        // Another block, of the same entity type or none: the entity is made again, bare.
        let out = w.edit_runs(
            &[Run {
                x: 1,
                z: 1,
                y0: 5,
                y1: 5,
                state: 1,
                only: None,
            }],
            |_, _| true,
            |s| s == 3,
        );
        let made = |n: &str| {
            (n == "machines:press").then(|| {
                BlockEntity::Mod(Box::new(sunrise_level::containers::ModBlockEntity::new(
                    "machines:press".into(),
                    Default::default(),
                )))
            })
        };
        replace_block_entities(&mut w, &out.tracked, names, made, None);
        assert!(w.block_entities.get(&(1, 5, 1)).is_none());
    }

    #[test]
    fn a_new_block_gets_the_block_entity_made_for_it() {
        let mut w = world();
        let out = w.edit_runs(
            &[Run {
                x: 3,
                z: 3,
                y0: 10,
                y1: 10,
                state: 3,
                only: None,
            }],
            |_, _| true,
            |s| s == 3,
        );
        // a mod block's, as `Game::new_block_entity` makes it
        let made = |n: &str| {
            (n == "machines:press").then(|| {
                BlockEntity::Mod(Box::new(sunrise_level::containers::ModBlockEntity::new(
                    "machines:press".into(),
                    Default::default(),
                )))
            })
        };
        replace_block_entities(&mut w, &out.tracked, names, made, None);
        assert!(matches!(
            w.block_entities.get(&(3, 10, 3)),
            Some(BlockEntity::Mod(e)) if e.id() == "machines:press"
        ));
    }

    /// `LevelChunk.setBlockState` keeps an entity only while
    /// `BlockEntity.isValidBlockState` holds: a stale one goes for the
    /// block's own (a sculk listener spilled under a same-name write), and
    /// a fitting one survives a same-name write (a chest's items stay).
    #[test]
    fn a_kept_block_entity_must_fit_its_block() {
        let sculk = BlockEntity::for_block("minecraft:sculk_sensor").expect("a sensor's");
        let chest = BlockEntity::for_block("minecraft:chest").expect("a chest's");
        assert!(!sculk.fits("minecraft:stone"));
        assert!(chest.fits("minecraft:chest"));
        // a 27-slot container never fits a hopper's five
        assert!(!chest.fits("minecraft:hopper"));

        let mut w = world();
        w.set(1, 5, 1, BlockStateId(2)).unwrap();
        w.block_entities.insert((1, 5, 1), sculk);
        let out = w.edit_runs(
            &[Run {
                x: 1,
                z: 1,
                y0: 5,
                y1: 5,
                state: 4,
                only: None,
            }],
            |_, _| true,
            |_| true,
        );
        assert_eq!(out.tracked, [((1, 5, 1), 2, 4)]);
        replace_block_entities(&mut w, &out.tracked, names, BlockEntity::for_block, None);
        assert!(
            matches!(
                w.block_entities.get(&(1, 5, 1)),
                Some(BlockEntity::Chest(_))
            ),
            "the chest's own entity replaces the spilled sculk one"
        );

        w.set(2, 5, 1, BlockStateId(2)).unwrap();
        w.block_entities.insert((2, 5, 1), chest);
        let out = w.edit_runs(
            &[Run {
                x: 2,
                z: 1,
                y0: 5,
                y1: 5,
                state: 4,
                only: None,
            }],
            |_, _| true,
            |_| true,
        );
        assert_eq!(out.tracked, [((2, 5, 1), 2, 4)]);
        replace_block_entities(&mut w, &out.tracked, names, BlockEntity::for_block, None);
        assert!(matches!(
            w.block_entities.get(&(2, 5, 1)),
            Some(BlockEntity::Chest(_))
        ));
    }

    #[test]
    fn each_heightmap_has_its_own_height() {
        let w = columns_world();
        assert_eq!(height(&w, Heightmap::WorldSurface, 1, 1), 65);
        assert_eq!(height(&w, Heightmap::MotionBlocking, 1, 1), 65);
        assert_eq!(height(&w, Heightmap::MotionBlockingNoLeaves, 1, 1), 63);
        assert_eq!(height(&w, Heightmap::OceanFloor, 1, 1), 65);
        assert_eq!(height(&w, Heightmap::OceanFloor, 2, 2), 63);
        assert_eq!(height(&w, Heightmap::MotionBlockingNoLeaves, 2, 2), 65);
        // an empty column, and one not loaded: the bottom
        assert_eq!(height(&w, Heightmap::OceanFloor, 5, 5), -64);
        assert_eq!(height(&w, Heightmap::WorldSurface, 100, 100), -64);
    }

    #[test]
    fn column_blocks_read_air_past_the_column() {
        let w = columns_world();
        let chunk = w.chunks.get(&(0, 0)).unwrap();
        let mut out = Vec::new();
        fill_column_blocks(w.air, chunk, w.shape, 1, 1, 65, 2, 3, &mut out);
        fill_column_blocks(w.air, chunk, w.shape, 3, 3, 319, 2, 3, &mut out);
        let read: Vec<u32> = out.iter().map(|s| s.0).collect();
        // (`names` calls state 2 a chest: here it stands for leaves)
        let (leaves, stone, air) = (2, 1, 0);
        assert_eq!(
            read,
            [
                leaves, leaves, air, air, air, // (1, 1) from 63
                air, air, stone, air, air, // (3, 3) from 317: stone at 319, nothing over 319
            ]
        );
    }

    /// The Nether's build range is 0..256 inside storage padded to the overworld's: a column
    /// there starts at 0, reads air outside its range, and spans at most 256 rows.
    #[test]
    fn a_shorter_level_bounds_columns_by_its_own_range() {
        let mut w = columns_world();
        w.shape = LevelShape::NETHER;
        assert_eq!(height(&w, Heightmap::OceanFloor, 5, 5), 0);
        assert_eq!(height(&w, Heightmap::WorldSurface, 100, 100), 0);
        let chunk = w.chunks.get(&(0, 0)).unwrap();
        let mut out = Vec::new();
        // Rows -2..=2: the two below the level read air, the rest are the stone below 60.
        fill_column_blocks(w.air, chunk, w.shape, 1, 1, 1, 3, 2, &mut out);
        let read: Vec<u32> = out.iter().map(|s| s.0).collect();
        assert_eq!(read, [0, 0, 1, 1, 1]);
        assert_eq!(column_span(0, 256, w.shape.height), Some(256));
        assert_eq!(column_span(0, 257, w.shape.height), None);
    }

    #[test]
    fn column_span_matches_level_height_only() {
        assert_eq!(column_span(200, 184, 384), Some(384));
        assert_eq!(column_span(200, 185, 384), None);
        assert_eq!(column_span(0, 0, 384), None);
        assert_eq!(column_span(u32::MAX, 1, 4064), None);
    }

    #[test]
    fn each_new_block_gets_its_own_block_entity() {
        // a mod's constructor numbering its instances; stone makes none
        let mut calls = Vec::new();
        let made = make_block_entities(
            &[
                "machines:press",
                "minecraft:stone",
                "machines:press",
                "minecraft:stone",
            ]
            .map(Arc::<str>::from),
            |n| {
                calls.push(n.to_string());
                (n.as_ref() == "machines:press").then(|| {
                    BlockEntity::Mod(Box::new(sunrise_level::containers::ModBlockEntity::new(
                        format!("press-{}", calls.len()),
                        Default::default(),
                    )))
                })
            },
        );
        let ids: Vec<Option<String>> = made
            .iter()
            .map(|e| match e {
                Some(BlockEntity::Mod(e)) => Some(e.id().to_string()),
                _ => None,
            })
            .collect();
        assert_eq!(
            ids,
            [
                Some("press-1".to_string()),
                None,
                Some("press-3".to_string()),
                None
            ]
        );
        assert_eq!(
            calls,
            ["machines:press", "minecraft:stone", "machines:press"]
        );
    }
    #[test]
    fn null_factories_are_cached_per_state_not_per_block_name() {
        // One block can create no BE in one state and a BE in another.
        // A cached null for the first state must not suppress the second.
        let mut calls = Vec::new();
        let made = make_block_entities(&[7_u32, 8, 7, 8], |state| {
            calls.push(*state);
            (*state == 8).then(|| {
                BlockEntity::Mod(Box::new(sunrise_level::containers::ModBlockEntity::new(
                    "machines:heater".into(),
                    Default::default(),
                )))
            })
        });
        assert_eq!(calls, [7, 8, 8]);
        assert!(made[0].is_none() && made[2].is_none());
        assert!(made[1].is_some() && made[3].is_some());
    }
}
