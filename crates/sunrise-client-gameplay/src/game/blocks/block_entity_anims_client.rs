use crate::game::ClientGame;
use crate::game::client_side::block_entity_anims_common_client::AnimTick;
use sunrise_client::{
    block_entity_anim::{BellEntity, ConduitTarget, Ticker},
    skins,
};
use sunrise_core::geometry::{Aabb, Vec3};
use sunrise_gameplay_common::game::block_entity_anims_common::PLAYER;
use sunrise_level::containers::BlockEntity;
use sunrise_level::particle_options::{ParticleOptions, ParticleType};

impl ClientGame<'_> {
    /// `SkullBlockRenderer.getPlayerSkin`: the skin sprite of the profile of
    /// the player head at `pos`. The skin download is started here; while it
    /// is in flight (or when it cannot be got) the profile's default skin
    /// stands in. A head with no profile answers None: `SKIN_BY_TYPE`.
    pub(in crate::game::client_side) fn skull_skin_at(&self, pos: [i32; 3]) -> Option<String> {
        let Some(BlockEntity::Skull(skull)) = self
            .common
            .world
            .block_entities
            .get(&(pos[0], pos[1], pos[2]))
        else {
            return None;
        };
        self.skull_skin(skull)
    }

    pub(in crate::game) fn skull_skin(
        &self,
        skull: &sunrise_level::containers::Skull,
    ) -> Option<String> {
        let skin = skull.skin.as_ref()?;
        // PlayerSkin.Patch: a `texture` override replaces the profile's skin.
        if let Some(texture) = &skin.texture
            && let Some(sprite) = skins::patch_skin(
                texture,
                &mut self.client.skin_downloads.0.borrow_mut(),
                |location| self.client.gui.atlas.sprites.contains_key(location),
            )
        {
            return Some(sprite);
        }
        // The texture a packed or resolved `textures` property fetches: the
        // download of a static profile, or the one the services answer for a
        // `Dynamic` (name- or id-only) profile.
        let download = skin.download.clone().or_else(|| {
            let key = skin.dynamic.as_ref()?;
            let skins::Resolve::Done(Some(profile)) =
                self.client.skin_profiles.borrow_mut().state(key)
            else {
                return None;
            };
            let url = skins::unpack_insecure(profile.textures()?).skin?;
            let location = skins::skin_location(&skins::texture_hash(&url)?);
            Some((url, location))
        });
        if let Some((url, location)) = download {
            let mut downloads = self.client.skin_downloads.0.borrow_mut();
            if self.client.gui.atlas.sprites.contains_key(&location) {
                let _ = downloads.state(skins::Kind::Skin, &url);
                downloads.drawn(&location);
                return Some(location);
            }
            // Its slot was evicted: offer the image again.
            if downloads.state(skins::Kind::Skin, &url) == skins::Download::Ready {
                downloads.again(&location);
            }
        }
        Some(skin.fallback.to_owned())
    }
    /// `BellBlockEntity.updateEntities` on the client: entities within 48 blocks.
    pub(in crate::game) fn client_bell_gather(&self, pos: [i32; 3]) -> Vec<u64> {
        let area = Aabb::new(
            Vec3::new(
                f64::from(pos[0]) - 48.0,
                f64::from(pos[1]) - 48.0,
                f64::from(pos[2]) - 48.0,
            ),
            Vec3::new(
                f64::from(pos[0]) + 49.0,
                f64::from(pos[1]) + 49.0,
                f64::from(pos[2]) + 49.0,
            ),
        );
        let mut out = Vec::new();
        if self
            .client_player_view()
            .is_some_and(|p| p.bounds.intersects(area))
        {
            out.push(PLAYER);
        }
        for m in &self.common.mobs {
            if !m.removed && m.bounds().intersects(area) {
                out.push(u64::from(m.id));
            }
        }
        out
    }
    /// `BellBlockEntity.clientTick`: resonance particles on the client copy.
    pub(in crate::game) fn client_tick_bell(
        &mut self,
        pos: [i32; 3],
        tags: &sunrise_items::items::EntityTypeTags,
    ) {
        let player = self.client_player_view().map(|p| BellEntity {
            position: [p.position.x, p.position.y, p.position.z],
            alive: p.alive,
            raider: false,
        });
        let mobs = &self.common.mobs;
        let entity = |h: u64| {
            if h == PLAYER {
                return player;
            }
            let m = mobs.iter().find(|m| u64::from(m.id) == h)?;
            Some(BellEntity {
                position: [m.position.x, m.position.y, m.position.z],
                alive: m.alive() && !m.removed,
                raider: tags.has_bit(
                    m.kind.flags.entity_tag_slot,
                    sunrise_items::items::EntityTypeTags::RAIDERS,
                ),
            })
        };
        let tick = self
            .client
            .be_anims
            .bells
            .entry(pos)
            .or_default()
            .tick(pos, entity);
        if let Some((particles, _)) = tick.ended {
            for (p, color) in particles {
                // ColorParticleOption.create(ENTITY_EFFECT, color): ARGB.
                let c = color as u32;
                let channel = |shift: u32| ((c >> shift) & 255) as f32 / 255.0;
                let options = ParticleOptions {
                    color: [channel(16), channel(8), channel(0), channel(24)],
                    ..ParticleOptions::simple(ParticleType::EntityEffect)
                };
                self.client.particles.0.add(&options, p, [0.0; 3]);
            }
        }
    }
    /// Client conduit target resolution from `BlockEntity::Conduit(uuid)` + remote UUID table.
    fn client_conduit_target(&self, pos: [i32; 3]) -> Option<ConduitTarget> {
        let Some(BlockEntity::Conduit(Some(uuid))) = self
            .common
            .world
            .block_entities
            .get(&(pos[0], pos[1], pos[2]))
        else {
            return None;
        };
        let u = sunrise_sim::lan::playerdata::ints_uuid(*uuid);
        let id = self
            .client
            .remote
            .as_ref()
            .and_then(|r| r.level_uuids.get(&u).copied())
            .map(|id| id as u32)
            .or_else(|| {
                self.common
                    .persist
                    .uuids
                    .iter()
                    .find(|&(_, v)| v == uuid)
                    .map(|(&id, _)| id)
            })?;
        let m = self.common.mobs.iter().find(|m| m.id == id && !m.removed)?;
        Some(ConduitTarget {
            position: [m.position.x, m.position.y, m.position.z],
            eye_y: m.eye_y(),
            width: m.width(),
            height: m.height(),
        })
    }
    /// The client ticker of the animated block entity at `pos` (`ClientLevel.tickBlockEntities`),
    /// in the ticker list's order with every other client ticker.
    pub(in crate::game) fn tick_block_entity_anim(
        &mut self,
        pos: [i32; 3],
        ticker: Ticker,
        anim: &mut AnimTick,
    ) {
        self.claim_anim(pos);
        {
            match ticker {
                Ticker::Lid => self.tick_chest_lid(pos),
                Ticker::Book => {
                    // The player where earlier tickers (a lid's push) left it.
                    let player = match &self.common.player {
                        Some(p) if !self.common.spectator() => {
                            Some([p.position.x, p.position.y, p.position.z])
                        }
                        _ => None,
                    };
                    let anims = &mut self.client.be_anims;
                    anims.books.entry(pos).or_default().tick(
                        pos,
                        player.as_slice(),
                        &mut anims.book_random,
                    );
                }
                Ticker::Shulker { facing } => {
                    // `ShulkerBoxBlockEntity.tick` runs on both sides: the
                    // client moves its own player, including with a menu open.
                    // The server ticks its boxes in `tick_block_entities`.
                    if !self.common.is_logical_server() {
                        let tick = self
                            .client
                            .be_anims
                            .shulkers
                            .entry(pos)
                            .or_default()
                            .tick(pos, facing);
                        // The lid pushes at its turn, so later tickers read the pushed player.
                        if let Some(push) = tick.push {
                            if let Some(shulker) = self.client.be_anims.shulkers.get(&pos) {
                                self.common.world.shulkers.insert(pos, shulker.clone());
                            }
                            self.push_shulker_local(vec![(pos, facing, push)]);
                        }
                    }
                }
                Ticker::Skull { powered } => {
                    self.client
                        .be_anims
                        .skulls
                        .entry(pos)
                        .or_default()
                        .tick(powered);
                }
                Ticker::Bell => {
                    // Server bells tick independently; clients animate their copy.
                    if !self.common.is_logical_server() {
                        let tags = self.items().entity_type_tags.clone();
                        self.client_tick_bell(pos, &tags);
                    }
                }
                Ticker::Conduit => {
                    let frames = anim.frames.get_or_insert_with(|| {
                        self.common
                            .res
                            .items()
                            .block_tags
                            .get("minecraft:conduit_effect_block")
                            .cloned()
                            .unwrap_or_default()
                    });
                    let target = self.client_conduit_target(pos);
                    let world = &self.common.world;
                    let facts = self.common.res.facts();
                    let registry = self.common.pack.assets.registry;
                    let is_water = |p: [i32; 3]| {
                        facts.get(world.block(p[0], p[1], p[2])).fluid
                            == sunrise_items::blocks::Fluid::Water
                    };
                    let is_frame = |p: [i32; 3]| {
                        registry
                            .state(world.block(p[0], p[1], p[2]))
                            .is_some_and(|s| frames.contains(&s.name))
                    };
                    let time = world.game_time;
                    let anims = &mut self.client.be_anims;
                    let particles = anims.conduits.entry(pos).or_default().client_tick(
                        pos,
                        time,
                        is_water,
                        is_frame,
                        target,
                        self.client.particles.0.level_random(),
                    );
                    let nautilus = ParticleOptions::simple(ParticleType::Nautilus);
                    for (p, v) in particles {
                        self.client.particles.0.add(&nautilus, p, v);
                    }
                }
                Ticker::Pot => {}
                Ticker::Gateway => {
                    // The server's portalTick (`tick_end_gateway`) already ran:
                    // its block event reaches this copy before it ticks.
                    let initial = self.common.gateway_loaded(pos);
                    self.client
                        .be_anims
                        .gateways
                        .entry(pos)
                        .or_insert(initial)
                        .tick();
                }
            }
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
impl ClientGame<'_> {
    pub fn test_conduit_target(&self, pos: [i32; 3]) -> Option<ConduitTarget> {
        self.client_conduit_target(pos)
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

#[cfg(test)]
pub(in crate::game) mod bell_ownership_tests {
    use sunrise_client::block_entity_anim::{Bell, BlockEntityAnims};

    #[test]
    fn server_bells_survive_visibility_changes_but_remote_bells_do_not() {
        let pos = [8, 200, 8];
        let mut anims = BlockEntityAnims::default();
        let mut bell = Bell::default();
        bell.trigger_event(1, 2, 100, || vec![7]);
        anims.bells.insert(pos, bell);
        anims.pots.insert(pos, Default::default());
        anims.retain_present_preserving_bells(&[]);
        assert_eq!(anims.bells[&pos].nearby.as_deref(), Some([7].as_slice()));
        assert!(anims.bells[&pos].shaking);
        assert!(anims.pots.is_empty());
        anims.retain_present(&[]);
        assert!(anims.bells.is_empty());
    }
}
