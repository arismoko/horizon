//! The player's advancements in the game: criteria triggered from game
//! events (`CriteriaTriggers`), awarding with rewards, the toast and the
//! chat announcement, and `players/advancements/<uuid>.json`.
use super::{Dim, chat_common::PLAYER_NAME, stats::Stats};
use crate::game::server_side::lan_actions_server::DisplacedPlayer;
pub mod icons;
use mod_api::{GameMode, ProgressType};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use sunrise_core::geometry::Vec3;
use sunrise_core::random::RandomSource;
use sunrise_game_data::text::{self, Style, Text};
use sunrise_gameplay_common::game::advancements_common::{TagSources, text_of};
use sunrise_items::items::ItemStack;
use sunrise_level::effects::PlayerEffects;
use sunrise_protocol::packets::play::Clientbound as C;
use sunrise_sim::{
    advancements::{self as adv, EntityView, Frame, ItemView, Progress, Tree},
    motion::PlayerMotion,
};

/// Capture the owning progress without changing its dates or requirement semantics.
pub(in crate::game::server_side) fn progress_snapshot(
    a: &adv::Advancement,
    progress: &Progress,
) -> mod_api::AdvancementProgress {
    let obtained = progress.criteria.get(&a.id);
    mod_api::AdvancementProgress::new(
        a.criteria
            .iter()
            .map(|criterion| {
                mod_api::CriterionProgress::new(
                    criterion.name.clone(),
                    obtained
                        .and_then(|criteria| criteria.get(&criterion.name))
                        .cloned(),
                )
            })
            .collect(),
        a.requirements.clone(),
        progress.counts(a).0 as u32,
        progress.done(a),
    )
}

/// The live parts of a player's entity predicate view, wherever the turn keeps them.
pub(in crate::game) struct PlayerEntityState<'a> {
    pub(in crate::game) body: &'a PlayerMotion,
    pub(in crate::game) name: &'a str,
    pub(in crate::game) mode: GameMode,
    pub(in crate::game) input_sneak: bool,
    pub(in crate::game) main_hand: Option<&'a ItemStack>,
    pub(in crate::game) off_hand: Option<&'a ItemStack>,
    pub(in crate::game) armor: &'a [Option<ItemStack>; 4],
    pub(in crate::game) effects: &'a PlayerEffects,
    pub(in crate::game) riding: Option<u32>,
    pub(in crate::game) look: Option<(f32, f32)>,
}

/// The game's world for predicates.
pub struct View<'a, 'g>(pub &'a crate::game::ServerGame<'g>);
impl adv::World for View<'_, '_> {
    fn is_loaded(&self, [x, y, z]: [i32; 3]) -> bool {
        self.0.world.is_loaded_at(x, y, z)
    }
    fn predicate(&self, id: &str) -> Option<&Value> {
        self.0.server.fns.0.store.predicates.get(id)
    }
    fn dimension(&self) -> &str {
        self.0.dim.id()
    }
    /// `WeatherCheck.test`'s level queries, using dimension-aware effective levels.
    fn weather(&self) -> (bool, bool) {
        let (rain, thunder) = self.0.weather_levels(1.0);
        (f64::from(rain) > 0.2, f64::from(thunder) > 0.9)
    }
    fn biome(&self, p: [i32; 3]) -> String {
        self.0
            .world
            .chunks
            .get(&(p[0] >> 4, p[2] >> 4))
            .map(|c| {
                self.0
                    .res
                    .biomes()
                    .get(c.biome(p[0], p[1], p[2]))
                    .name
                    .clone()
            })
            .unwrap_or_default()
    }
    fn structures(&self, p: [i32; 3]) -> Vec<String> {
        self.0.structures_at(p)
    }
    fn block_state(&self, p: [i32; 3]) -> Option<&sunrise_world::chunk::BlockState> {
        self.0
            .pack
            .assets
            .registry
            .state(self.0.world.block(p[0], p[1], p[2]))
    }
    fn block(&self, p: [i32; 3]) -> (String, BTreeMap<String, String>) {
        let state = self.0.world.block(p[0], p[1], p[2]);
        let name = self.0.res.facts().get(state).name.to_string();
        let props = self
            .0
            .pack
            .assets
            .registry
            .state(state)
            .map(|s| {
                s.properties
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        (name, props)
    }
    fn block_property_values(&self, block: &str, property: &str) -> Option<Vec<String>> {
        let values = self.0.pack.assets.registry.property_values(block, property);
        (!values.is_empty()).then(|| values.into_iter().map(str::to_owned).collect())
    }
    fn fluid(&self, p: [i32; 3]) -> Option<String> {
        match self
            .0
            .res
            .facts()
            .get(self.0.world.block(p[0], p[1], p[2]))
            .fluid
        {
            sunrise_items::blocks::Fluid::Water => Some("minecraft:water".into()),
            sunrise_items::blocks::Fluid::Lava => Some("minecraft:lava".into()),
            _ => None,
        }
    }
    /// `Level.canSeeSky`: sky light 15 at the position, whatever stands
    /// above it, as `MobWorld` reads it for the spawn rules.
    fn can_see_sky(&self, p: [i32; 3]) -> bool {
        self.0.mob_world_base(&[]).can_see_sky(p[0], p[1], p[2])
    }
    fn light(&self, p: [i32; 3]) -> i32 {
        let l = self.0.world.light(p[0], p[1], p[2]);
        i32::from((l >> 4).max(l & 15))
    }
    /// `CampfireBlock.isSmokeyPos`: share the beehive smoke test.
    fn smokey(&self, p: [i32; 3]) -> bool {
        sunrise_sim::beehive::smokey(&self.0.mob_world_base(&[]), p)
    }
    /// The same `saveWithFullMetadata` a loot roll's `match_block` reads: the live block
    /// entity over the NBT it was loaded from, else its saved row.
    fn block_entity_nbt(&self, p: [i32; 3]) -> Option<sunrise_core::nbt::Tag> {
        self.0.block_entity_nbt(p).or_else(|| {
            sunrise_gameplay_common::game::spill_common::saved_full_metadata(&self.0.world, p)
        })
    }
    fn in_tag(&self, registry: &str, tag: &str, id: &str) -> bool {
        let tags = TagSources {
            pack_tags: self.0.server.world_tables.0.as_ref().map(|t| &t.pack_tags),
            items: self.0.items(),
            mob_data: self.0.res.mob_data(),
            damage: &self.0.health_resources.damage,
            resources: &self.0.options.resources,
        };
        tags.contains(registry, tag, id)
    }
}
pub(in crate::game::server_side) fn now_text() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs()) as i64;
    adv::date_text(secs)
}
impl crate::game::ServerGame<'_> {
    /// `ItemPredicate.test` (the `item_filter` of a filtered item modifier).
    pub fn loot_item_filter(&self, predicate: &Value, s: &ItemStack) -> bool {
        adv::item(predicate, &item_view(s), &View(self))
    }
    /// `BaseContainerBlockEntity.canOpen`: whether the `lock` of the block
    /// entity at `key` keeps a player with this main hand out (`LockCode`).
    pub(in crate::game) fn container_locked_for(
        &self,
        key: (i32, i32, i32),
        spectator: bool,
        hand: Option<&ItemStack>,
    ) -> bool {
        let Some(lock) = self.common.world.block_lock(key) else {
            return false;
        };
        let lock = sunrise_sim::item_models::tag_json(lock);
        let hand = hand.map(item_view);
        !adv::can_unlock(&lock, spectator, hand.as_ref(), &View(self))
    }
}
/// `MinMaxBounds.Doubles.matchesSqr`: `sqr` against the bounds squared
/// (a missing one matches anything).
pub(in crate::game::server_side) fn matches_sqr(v: &Value, sqr: f64) -> bool {
    let (min, max) = match v {
        Value::Null => (None, None),
        Value::Number(n) => (n.as_f64(), n.as_f64()),
        Value::Object(o) => (
            o.get("min").and_then(Value::as_f64),
            o.get("max").and_then(Value::as_f64),
        ),
        _ => return false,
    };
    !min.is_some_and(|m| m * m > sqr) && !max.is_some_and(|m| m * m < sqr)
}
pub(in crate::game) fn item_view(s: &ItemStack) -> ItemView {
    ItemView {
        id: s.item.to_string(),
        count: s.count,
        damage: s.damage,
        enchantments: s
            .enchantments()
            .iter()
            .map(|(id, l)| (id.to_string(), *l))
            .collect(),
        potion: s.potion().map(str::to_owned),
        custom_name: s.custom_name().is_some(),
        custom_data: None,
    }
}
impl crate::game::ServerGame<'_> {
    pub(in crate::game::server_side) fn adv_path(&self) -> Option<std::path::PathBuf> {
        Some(super::stats::player_json(
            &self.server.world_root,
            "advancements",
            self.common.level.player_uuid?,
        ))
    }
    /// A server with no player of its own and nobody it acts for: there is
    /// no player for advancements and statistics.
    pub(in crate::game::server_side) fn no_player_here(&self) -> bool {
        self.common.options.dedicated.is_some() && self.server.lan_acting.is_none()
    }
    /// Load the local tree once; an empty server tree is still authoritative.
    pub fn advancement_tree(&mut self) -> Arc<Tree> {
        if self.common.adv.remote_tree.is_none() && self.common.adv.tree.list.is_empty() {
            let overlay = self.pack_files();
            self.common.adv.tree = if self.server.fns.0.store.predicates.is_empty() {
                Tree::shared(&self.common.options.resources.join("data"), &overlay)
            } else {
                let predicates: std::collections::HashMap<String, Value> = self
                    .server
                    .fns
                    .0
                    .store
                    .predicates
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                Arc::new(Tree::load_with_predicates(
                    &self.common.options.resources.join("data"),
                    &overlay,
                    &predicates,
                ))
            };
            self.inventory_index();
        }
        self.common.adv.tree.clone()
    }
    /// `inventory_changed`'s criteria by item, for the tree in place.
    pub(in crate::game::server_side) fn inventory_index(&mut self) -> Arc<adv::InventoryIndex> {
        let tree = self.common.adv.tree.clone();
        if let Some((built, index)) = &self.common.adv.inventory_index
            && Arc::ptr_eq(built, &tree)
        {
            return index.clone();
        }
        let items = self.items();
        let index = Arc::new(adv::InventoryIndex::build(&tree, |tag| {
            items
                .item_tags
                .get(tag)
                .map(|t| t.iter().map(String::as_str).collect())
                .unwrap_or_default()
        }));
        self.common.adv.inventory_index = Some((tree, index.clone()));
        index
    }
    pub(in crate::game) fn load_advancements(&mut self) {
        if self.common.adv.loaded || self.no_player_here() {
            return;
        }
        self.common.adv.loaded = true;
        self.advancement_tree();
        let progress = self
            .adv_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .map(|v| Progress::from_json(&v))
            .unwrap_or_default();
        // The known recipes are the player's own (`recipeBook`), not the
        // rewards of done advancements: /recipe take sticks.
        let tree = self.common.adv.tree.clone();
        self.common.adv.progress = progress;
        self.common.ui.registries.advancements = tree.list.iter().map(|a| a.id.clone()).collect();
    }
    pub fn save_advancements(&self) {
        if !self.common.adv.loaded
            || self.no_player_here()
            || self
                .server
                .crash_save
                .holds_interrupted(self.server.lan_acting)
        {
            return;
        }
        let Some(path) = self.adv_path() else {
            return;
        };
        let json = self
            .common
            .adv
            .progress
            .to_json(&self.common.adv.tree, sunrise_world::DATA_VERSION);
        if let Ok(text) = serde_json::to_string_pretty(&json) {
            let _ = sunrise_sim::save::write_durable(&path, text.as_bytes());
        }
    }
    /// The player as predicates see it.
    /// The predicate view of a command target.
    pub fn entity_view_of(&self, t: super::commands::Target) -> EntityView {
        use super::commands::Target;
        match t {
            Target::Player => self.player_entity(),
            Target::Mob(id) => self
                .common
                .mobs
                .iter()
                .find(|m| m.id == id)
                .map(|m| self.mob_entity(m))
                .unwrap_or_default(),
            // A server's player: what its last `ServerboundPlayerInputPacket`
            // held (shift 32) and its game mode.
            Target::Guest(conn) if self.command_guest(conn).is_some() => {
                let g = self.command_guest(conn).expect("checked");
                EntityView {
                    kind: self.target_type(t),
                    position: g.position,
                    sneaking: g.sneaking,
                    input_sneak: g.input & 32 != 0,
                    team: self
                        .common
                        .score
                        .board
                        .players_team(&g.name)
                        .map(|t| t.name.clone()),
                    gamemode: Some(
                        match g.game_mode {
                            1 => "creative",
                            2 => "adventure",
                            3 => "spectator",
                            _ => "survival",
                        }
                        .to_owned(),
                    ),
                    ..EntityView::default()
                }
            }
            other => EntityView {
                kind: self.target_type(other),
                position: self.target_position(other).unwrap_or_default(),
                ..EntityView::default()
            },
        }
    }
    /// A `ContextAwarePredicate` for `this` at `origin`.
    pub fn conditions_hold(&self, v: &Value, this: Option<&EntityView>, origin: [f64; 3]) -> bool {
        self.conditions_hold_in(
            v,
            &adv::Entities {
                this,
                origin,
                tool: None,
                event_block: None,
                target: None,
                interacting: None,
            },
        )
    }
    /// A `ContextAwarePredicate` in a loot context that may also carry
    /// `target_entity` and `interacting_entity`.
    pub(in crate::game::server_side) fn conditions_hold_in(
        &self,
        v: &Value,
        entities: &adv::Entities,
    ) -> bool {
        adv::conditions(v, entities, &View(self))
    }
    /// `ExecuteCommand.checkCustomPredicate`: no seed/sequence override;
    /// reached literal chance leaves use this active level's random source.
    pub(in crate::game::server_side) fn command_conditions_hold(
        &mut self,
        v: &Value,
        this: Option<&EntityView>,
        origin: [f64; 3],
    ) -> bool {
        let ctx = adv::Entities {
            this,
            origin,
            tool: None,
            event_block: None,
            target: None,
            interacting: None,
        };
        // An invalid reference anywhere fails the whole tree, even under an inversion.
        if !adv::references_valid(v, &View(self)) {
            return false;
        }
        adv::conditions_with(v, &ctx, &mut |leaf, ctx| {
            adv::literal_random_chance(leaf, &mut || self.common.world.random.next_float())
                .unwrap_or_else(|| adv::conditions(leaf, ctx, &View(self)))
        })
    }
    /// The player as a command's predicates see it: with what it looks at.
    pub(in crate::game::server_side) fn player_entity(&self) -> EntityView {
        self.player_entity_looking(true)
    }
    /// [`Self::entity_view_of`] for testing `predicate` (a predicate, a list
    /// of conditions or an item modifier): the player with the level,
    /// statistics, advancements and recipes it may ask for, and nothing
    /// copied when it asks for none.
    pub(in crate::game::server_side) fn entity_view_asked(
        &self,
        t: super::commands::Target,
        predicate: &Value,
    ) -> EntityView {
        self.entity_view_asking(t, adv::facts_asked_in(predicate))
    }
    /// [`Self::entity_view_asked`] with the holder's live scoreboard scores,
    /// which `entity_scores` reads.
    pub(in crate::game::server_side) fn entity_view_scored(
        &mut self,
        t: super::commands::Target,
        predicate: &Value,
    ) -> EntityView {
        let mut view = self.entity_view_asked(t, predicate);
        view.scores = self.holder_name(t).map(|holder| {
            self.common
                .score
                .board
                .holder_scores(&holder)
                .into_iter()
                .map(|(objective, value)| (objective.name.clone(), value))
                .collect()
        });
        view
    }
    /// [`Self::entity_view_of`] with the player's records `asked` names.
    pub(in crate::game::server_side) fn entity_view_asking(
        &self,
        t: super::commands::Target,
        asked: adv::FactsAsked,
    ) -> EntityView {
        use super::commands::Target;
        let mut view = self.entity_view_of(t);
        match t {
            Target::Player => view.player_facts = self.player_facts(asked),
            Target::Guest(conn) => view.player_facts = self.guest_facts(conn, asked),
            _ => {}
        }
        view
    }
    /// [`Self::player_facts`] of a server's player who is not in place
    /// (selectors leave out the one in place): its own records, kept on the
    /// hub while another acts.
    pub(in crate::game::server_side) fn guest_facts(
        &self,
        conn: u32,
        asked: adv::FactsAsked,
    ) -> adv::PlayerFacts {
        let Some(host) = self.server.lan.as_ref() else {
            return adv::PlayerFacts::default();
        };
        let Some(g) = host
            .hub
            .guest(conn)
            .filter(|_| self.server.lan_acting != Some(conn))
        else {
            return adv::PlayerFacts::default();
        };
        let stats = (host.play.get(&conn).map(|p| &p.stats)).filter(|s| s.loaded);
        adv::PlayerFacts {
            level: asked.level.then_some(g.xp_level),
            // The guest's food as last written back from its body.
            food: Some((g.food, g.saturation)),
            stats: stats.filter(|_| asked.stats).map(|s| s.values.clone()),
            advancements: asked
                .advancements
                .then(|| (self.common.adv.tree.clone(), g.advancements.clone())),
            recipes: asked.recipes.then(|| g.recipe_book.recipes.clone()),
        }
    }
    /// [`Self::player_entity`] for `trigger`'s criteria: what the player
    /// looks at, and its level, statistics, advancements and recipes, only
    /// when one of them may ask (a ray against every entity and copies of
    /// the player's records otherwise), and its compound only when one of
    /// them may test `nbt`.
    pub(in crate::game) fn player_entity_for(&mut self, trigger: &str) -> EntityView {
        let mut me = self.player_entity_for_any(&[trigger]);
        if self.common.adv.tree.may_test_nbt(trigger) {
            me.nbt = self.player_nbt();
        }
        me
    }
    /// [`Self::player_entity_for`] for the criteria of several triggers,
    /// without the player's compound (building it needs `&mut self`).
    pub(in crate::game) fn player_entity_for_any(&self, triggers: &[&str]) -> EntityView {
        let tree = &self.common.adv.tree;
        let mut me = self.player_entity_looking(triggers.iter().any(|t| tree.may_look(t)));
        let asked = (triggers.iter()).fold(adv::FactsAsked::default(), |all, t| {
            all.or(tree.player_facts_asked(t))
        });
        me.player_facts = self.player_facts(asked);
        me
    }
    /// `KilledTrigger` takes the actual killer, not the player borrowing the acting slot.
    pub(in crate::game::server_side) fn displaced_player_entity_for(
        &self,
        frame: &DisplacedPlayer,
        body: &PlayerMotion,
        trigger: &str,
    ) -> Option<EntityView> {
        let record = self.server.lan.as_ref()?.hub.guest(frame.record_conn)?;
        // Vehicle and sight queries below use the installed level's entities.
        let here = frame.dimension == self.common.dim;
        let mut view = self.player_entity_from(PlayerEntityState {
            body,
            name: frame.name.as_deref().unwrap_or(PLAYER_NAME),
            mode: frame.mode,
            input_sneak: frame.input_sneak,
            main_hand: record
                .inventory
                .get(frame.selected)
                .and_then(Option::as_ref),
            off_hand: record.offhand.as_ref(),
            armor: &record.armor,
            effects: &record.effects,
            riding: frame.riding.filter(|_| here),
            look: (here && self.common.adv.tree.may_look(trigger))
                .then_some((frame.yaw, frame.pitch)),
        });
        view.player_facts = self.player_facts_from(
            self.common.adv.tree.player_facts_asked(trigger),
            body,
            frame.experience.level,
            &frame.stats,
            frame.adv_loaded.then_some(&record.advancements),
            &record.recipe_book.recipes,
        );
        Some(view)
    }
    pub(in crate::game::server_side) fn player_facts(
        &self,
        asked: adv::FactsAsked,
    ) -> adv::PlayerFacts {
        let Some(p) = self.common.player.as_ref() else {
            return adv::PlayerFacts::default();
        };
        self.player_facts_from(
            asked,
            p,
            self.common.experience.level,
            &self.common.stats,
            self.common.adv.loaded.then_some(&self.common.adv.progress),
            &self.common.level.recipe_book.recipes,
        )
    }
    /// Only copy the records named by `PlayerPredicate`; unloaded records stay absent.
    pub(in crate::game::server_side) fn player_facts_from(
        &self,
        asked: adv::FactsAsked,
        body: &PlayerMotion,
        level: i32,
        stats: &Stats,
        progress: Option<&Progress>,
        recipes: &BTreeSet<String>,
    ) -> adv::PlayerFacts {
        adv::PlayerFacts {
            level: asked.level.then_some(level),
            food: Some((body.health.food.food, body.health.food.saturation)),
            stats: (asked.stats && stats.loaded).then(|| stats.values.clone()),
            advancements: progress
                .filter(|_| asked.advancements)
                .map(|p| (self.common.adv.tree.clone(), p.clone())),
            recipes: asked.recipes.then(|| recipes.clone()),
        }
    }
    /// The player as predicates see it, with what it looks at when `looking`.
    pub(in crate::game::server_side) fn player_entity_looking(&self, looking: bool) -> EntityView {
        let Some(body) = self.common.player.as_ref() else {
            return EntityView::default();
        };
        self.player_entity_from(PlayerEntityState {
            body,
            name: self.acting_player_name(),
            mode: self.common.mode,
            input_sneak: self.server_acting_sneak(),
            main_hand: self.common.held(),
            off_hand: self.common.offhand.as_ref(),
            armor: &self.common.armor,
            effects: &self.common.potions.effects,
            riding: self.common.riding,
            look: looking.then_some((self.common.yaw, self.common.pitch)),
        })
    }
    pub(in crate::game::server_side) fn player_entity_from(
        &self,
        state: PlayerEntityState<'_>,
    ) -> EntityView {
        let p = state.body;
        let mut eq: [Option<ItemView>; 7] = Default::default();
        eq[0] = state.main_hand.map(item_view);
        eq[1] = state.off_hand.map(item_view);
        for (i, a) in state.armor.iter().enumerate() {
            eq[2 + i] = a.as_ref().map(item_view);
        }
        let vehicle = state.riding.and_then(|id| {
            self.common
                .mobs
                .iter()
                .find(|m| m.id == id)
                .map(|m| Box::new(self.mob_entity(m)))
                .or_else(|| {
                    self.common
                        .boats
                        .iter()
                        .find(|b| b.id == id)
                        .map(|b| Box::new(self.boat_entity(b)))
                })
        });
        EntityView {
            kind: "minecraft:player".into(),
            position: [p.position.x, p.position.y, p.position.z],
            on_fire: p.is_on_fire(),
            sneaking: p.pose == sunrise_sim::pose::Pose::Crouching,
            // Input.shift: the held key, as `BlockItem.place`'s sneaking reads it.
            input_sneak: state.input_sneak,
            sprinting: p.sprinting,
            swimming: p.pose == sunrise_sim::pose::Pose::Swimming,
            flying: p.flying,
            on_ground: p.ground.on_ground,
            equipment: eq,
            vehicle,
            effects: state
                .effects
                .list
                .iter()
                .map(|e| {
                    let id = sunrise_level::brewing::namespaced(e.id);
                    (id.to_owned(), e.amplifier, e.duration)
                })
                .collect(),
            gamemode: Some(state.mode.name().to_owned()),
            team: self
                .common
                .score
                .board
                .players_team(state.name)
                .map(|t| t.name.clone()),
            // Vanilla looks for it inside PlayerPredicate.matches whatever the
            // trigger; skipped for a trigger whose criteria can't ask.
            looking_at: state.look.and_then(|(yaw, pitch)| {
                let eye = [
                    p.position.x,
                    p.position.y + f64::from(p.dimensions.eye_height),
                    p.position.z,
                ];
                self.looking_at_entity(eye, super::view_vector(yaw, pitch))
                    .map(Box::new)
            }),
            ..Default::default()
        }
    }
    /// The player as a passenger of the vehicle `id` (`Entity.getPassengers`).
    pub(in crate::game::server_side) fn rider_entity(&self, id: u32) -> Option<EntityView> {
        let p = self
            .common
            .player
            .as_ref()
            .filter(|_| self.common.riding == Some(id))?;
        Some(EntityView {
            kind: "minecraft:player".into(),
            position: [p.position.x, p.position.y, p.position.z],
            gamemode: Some(self.common.mode.name().to_owned()),
            ..Default::default()
        })
    }
    /// A boat as the predicates see it: its type, and its passengers (the
    /// mobs seated on it, then the player riding it).
    pub(in crate::game::server_side) fn boat_entity(
        &self,
        b: &sunrise_gameplay_common::game::boats_common::Boat,
    ) -> EntityView {
        EntityView {
            kind: b.entity_type(),
            position: [b.position.x, b.position.y, b.position.z],
            on_ground: b.ground.on_ground,
            passengers: self
                .common
                .mobs
                .iter()
                .filter(|m| m.extra.mount == Some(b.id))
                .map(|m| self.mob_entity(m))
                .chain(self.rider_entity(b.id))
                .collect(),
            ..Default::default()
        }
    }
    /// A dropped item as the predicates see it.
    pub(in crate::game::server_side) fn item_entity_view(
        &self,
        e: &sunrise_sim::entities::ItemEntity,
    ) -> EntityView {
        EntityView {
            kind: "minecraft:item".into(),
            position: [e.position.x, e.position.y, e.position.z],
            on_ground: e.ground.on_ground,
            on_fire: e.fire_ticks > 0,
            ..Default::default()
        }
    }
    /// `Entity.get`: the mob's live custom-name component and type variants,
    /// alongside the other state the entity predicate asks for.
    pub fn mob_entity(&self, m: &sunrise_sim::mobs::Mob) -> EntityView {
        // mainhand, offhand, head, chest, legs, feet.
        let slot = |i: Option<&str>| i.map(|id| item_view(&ItemStack::new(id, 1)));
        let mut equipment: [Option<ItemView>; 7] = Default::default();
        equipment[0] = slot(m.extra.main_hand_id());
        equipment[1] = slot(m.extra.off_hand_id());
        equipment[2] = slot(m.extra.head_id());
        equipment[3] = slot(m.extra.slot_id(sunrise_items::equipment::CHEST));
        equipment[4] = slot(m.extra.slot_id(sunrise_items::equipment::LEGS));
        equipment[5] = slot(m.extra.slot_id(sunrise_items::equipment::FEET));
        equipment[6] = m.extra.body_stack().map(item_view);
        // Only the new snapshot is adopted; the existing view fields stay as
        // constructed above. Body and other view consumers stay unprojected.
        use sunrise_sim::mobs::finalize;
        for (view, slot) in equipment[..6].iter_mut().zip([
            finalize::MAINHAND,
            finalize::OFFHAND,
            finalize::HEAD,
            finalize::CHEST,
            finalize::LEGS,
            finalize::FEET,
        ]) {
            if let Some(view) = view {
                let data = finalize::item_stack(m, slot)
                    .and_then(|stack| stack.custom_data().cloned())
                    .unwrap_or_default();
                view.custom_data = Some(sunrise_core::nbt::Tag::Compound(data));
            }
        }

        let passengers: Vec<EntityView> = self
            .common
            .mobs
            .iter()
            .filter(|r| r.extra.mount == Some(m.id))
            .map(|r| self.mob_entity(r))
            .chain(self.rider_entity(m.id))
            .collect();
        let nbt = self.common.adv.tree.tests_nbt().then(|| {
            let uuid = self
                .common
                .persist
                .uuids
                .get(&m.id)
                .copied()
                .unwrap_or_default();
            let raw = self.common.persist.raw.get(&m.id);
            let registry = Some(self.common.pack.assets.registry);
            let mut tag = sunrise_sim::entity_nbt::mob_to_nbt(
                m,
                uuid,
                raw,
                registry,
                |other| self.common.persist.uuid_of(other),
                self.common.res.items(),
            );
            self.put_last_hurt_by_mob(m, &mut tag);
            let riders = passengers.iter().filter_map(|r| r.nbt.clone()).collect();
            super::persist::add_passengers(&mut tag, riders);
            tag
        });
        let movement = (m.extra.mount.is_none()
            && m.rider.is_none()
            && self.common.riding != Some(m.id)
            && self.server.lan.as_ref().is_none_or(|host| {
                host.hub
                    .guests
                    .iter()
                    .all(|guest| guest.vehicle != Some(m.id))
            }))
        .then_some(adv::MovementView {
            per_tick: [m.velocity.x, m.velocity.y, m.velocity.z],
            fall_distance: m.fall_distance,
        });
        EntityView {
            movement,
            kind: m.kind.id.clone(),
            position: [m.position.x, m.position.y, m.position.z],
            on_fire: m.fire_ticks > 0 && !m.mod_fire_immune,
            baby: m.is_baby(),
            on_ground: m.ground.on_ground,
            equipment,
            // LivingEntity.getActiveEffectsMap supplies the active predicate instances.
            effects: m
                .extra
                .effects
                .iter()
                .map(|effect| {
                    let (id, duration, amplifier) = (effect.id, effect.duration, effect.amplifier);
                    let id = sunrise_level::brewing::namespaced(id);
                    (id.to_owned(), amplifier, duration)
                })
                .collect(),
            components: sunrise_sim::mobs::finalize::variant_component_in(m, self.world_variants())
                .map(|(id, value)| (id.to_owned(), value))
                .into_iter()
                .collect(),
            custom_name: m.custom_name_text.as_ref().map(Text::component),
            passengers,
            nbt,
            team: self.mob_team_of(m).map(|t| t.name.clone()),
            ..Default::default()
        }
    }
    /// `SimpleCriterionTrigger.trigger`: test each open criterion of the
    /// trigger and award what it completes.
    pub fn trigger(&mut self, name: &str, mut test: impl FnMut(&Value, &dyn adv::World) -> bool) {
        self.trigger_custom(name, |c, view| test(&c.conditions, view));
    }
    /// `SimpleCriterionTrigger.trigger` with access to each loaded [`adv::Criterion`].
    pub(in crate::game) fn trigger_custom(
        &mut self,
        name: &str,
        test: impl FnMut(&adv::Criterion, &dyn adv::World) -> bool,
    ) {
        if self.server.lan_acting_fake || self.no_player_here() {
            return;
        }
        self.load_advancements();
        let tree = self.common.adv.tree.clone();
        let Some(list) = tree.by_trigger.get(name) else {
            return;
        };
        self.trigger_criteria_with(&tree, list, test);
    }
    /// `trigger` for some of a trigger's criteria: (advancement, criterion)
    /// indexes in `tree`, in tree order.
    pub(in crate::game::server_side) fn trigger_criteria(
        &mut self,
        tree: &Tree,
        list: &[(usize, usize)],
        mut test: impl FnMut(&Value, &dyn adv::World) -> bool,
    ) {
        self.trigger_criteria_with(tree, list, |c, view| test(&c.conditions, view));
    }
    pub(in crate::game::server_side) fn trigger_criteria_with(
        &mut self,
        tree: &Tree,
        list: &[(usize, usize)],
        mut test: impl FnMut(&adv::Criterion, &dyn adv::World) -> bool,
    ) {
        if self.server.lan_acting_fake {
            return;
        }
        let mut hits = Vec::new();
        {
            let view = View(self);
            for &(ai, ci) in list {
                let a = &tree.list[ai];
                let c = &a.criteria[ci];
                if self.common.adv.progress.has(&a.id, &c.name) || self.common.adv.progress.done(a)
                {
                    continue;
                }
                if test(c, &view) {
                    hits.push((ai, ci));
                }
            }
        }
        if hits.is_empty() {
            return;
        }
        let date = now_text();
        for (ai, ci) in hits {
            let a = &tree.list[ai];
            let was = self.common.adv.progress.done(a);
            let criterion = &a.criteria[ci].name;
            if self.common.adv.progress.grant(&a.id, criterion, &date) {
                self.advancement_progress(a, criterion, ProgressType::Grant);
            }
            self.progress_changed(ai);
            if !was && self.common.adv.progress.done(a) {
                self.award(ai);
            }
        }
    }
    /// `PlayerAdvancements.award` / `revoke` after a criterion's progress changed.
    pub(in crate::game::server_side) fn advancement_progress(
        &mut self,
        advancement: &adv::Advancement,
        criterion: &str,
        progress_type: ProgressType,
    ) {
        if !mod_api::__private::listened(mod_api::HookId::PlayerAdvancementProgress) {
            return;
        }
        if let (Some(uuid), Some(entity_id), Some(name)) = (
            self.server.lan_acting_uuid,
            self.server.lan_acting_entity,
            self.server.lan_acting_name.clone(),
        ) {
            let progress = progress_snapshot(advancement, &self.common.adv.progress);
            self.native_player_advancement_progress(
                mod_api::__private::PlayerAdvancementProgressCall {
                    player: uuid,
                    player_entity_id: entity_id as i32,
                    name: &name,
                    advancement: &advancement.id,
                    criterion,
                    progress_type,
                    progress: &progress,
                },
            );
        }
    }
    /// Lend the hub each guest turn's actual advancement storage, without
    /// moving it back into a record while that turn is still active.
    pub fn with_advancement_hub<R>(
        &mut self,
        run: impl FnOnce(
            &mut sunrise_sim::lan::hub::Hub,
            &mut sunrise_sim::lan::hub::advancements::AdvancementOwners<'_>,
        ) -> R,
    ) -> Option<R> {
        use sunrise_sim::lan::hub::advancements::{AdvancementOwners, AdvancementState};
        let host = self.server.lan.as_mut()?;
        let mut owners = AdvancementOwners::active(
            &self.server.lan_guest_swaps,
            AdvancementState {
                progress: &mut self.common.adv.progress,
                changed: &mut self.common.adv.changed,
                unlocked: &mut self.common.adv.unlocked,
                book: &mut self.common.level.recipe_book,
            },
        );
        Some(run(&mut host.hub, &mut owners))
    }

    /// Grant the hub's tick criteria one at a time: each hook sees the new
    /// progress before the completion's rewards and packets are applied.
    pub fn run_native_advancement_grants(&mut self) {
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        let tasks = std::mem::take(&mut host.hub.native_advancement_grants);
        for task in tasks {
            let mut fresh = Vec::new();
            for grant in &task.grants {
                let Some(was_done) = self
                    .with_advancement_hub(|hub, owners| hub.grant_native_tick(&task, grant, owners))
                    .flatten()
                else {
                    continue;
                };
                fresh.push(grant.clone());
                if mod_api::__private::listened(mod_api::HookId::PlayerAdvancementProgress) {
                    let definition = task
                        .sync
                        .advancements
                        .definition(&grant.advancement)
                        .expect("granted advancement");
                    let progress = self
                        .with_advancement_hub(|hub, owners| {
                            hub.advancement_task_progress(&task, owners, |progress| {
                                progress_snapshot(definition, progress)
                            })
                        })
                        .flatten()
                        .expect("granted task's progress");
                    self.native_player_advancement_progress(
                        mod_api::__private::PlayerAdvancementProgressCall {
                            player: task.player,
                            player_entity_id: task.entity_id,
                            name: &task.name,
                            advancement: &grant.advancement,
                            criterion: &grant.criterion,
                            progress_type: ProgressType::Grant,
                            progress: &progress,
                        },
                    );
                }
                let definition = task
                    .sync
                    .advancements
                    .definition(&grant.advancement)
                    .expect("granted advancement");
                let done = self
                    .with_advancement_hub(|hub, owners| {
                        hub.advancement_task_progress(&task, owners, |progress| {
                            progress.done(definition)
                        })
                    })
                    .flatten()
                    .unwrap_or(false);
                if self.server.lan_acting == Some(task.conn) {
                    if !was_done && done {
                        self.award_definition(definition);
                    }
                } else {
                    self.with_advancement_hub(|hub, owners| {
                        hub.reward_native_tick(&task, grant, was_done, owners);
                    });
                }
            }
            self.with_advancement_hub(|hub, owners| {
                hub.finish_native_first_tick(&task, &fresh, owners);
            });
        }
        let finished_reload = self
            .with_advancement_hub(|hub, owners| {
                if hub.native_reload_pending {
                    hub.finish_reload_resources(owners);
                    true
                } else {
                    false
                }
            })
            .unwrap_or(false);
        if finished_reload {
            self.send_datapack_snapshots();
        }
    }
    /// `PlayerAdvancements.progressChanged`, kept for a network player's client.
    pub(in crate::game::server_side) fn progress_changed(&mut self, index: usize) {
        if self.server.lan_acting.is_some() {
            self.common.adv.changed.insert(index);
        }
    }
    /// After `ServerRecipeBook.addRecipes`: the toast and the
    /// `recipe_unlocked` trigger of each recipe now known. Returns the
    /// number of recipe displays taken back out because a mod refused their
    /// discovery.
    pub(in crate::game::server_side) fn recipe_unlocked(&mut self, unlocked: &[String]) -> usize {
        let mut allowed = Vec::with_capacity(unlocked.len());
        let mut refused = Vec::new();
        let mut taken_back = 0;
        if let (Some(player), Some(name)) = (
            self.server.lan_acting_uuid,
            self.server.lan_acting_name.clone(),
        ) {
            for recipe in unlocked {
                let mut cancel = false;
                self.native_player_recipe_discovered(player, &name, recipe, &mut cancel);
                if cancel {
                    refused.push(recipe.clone());
                } else {
                    allowed.push(recipe.clone());
                }
            }
            if !refused.is_empty() {
                let removed = self
                    .common
                    .level
                    .recipe_book
                    .remove_recipes(&server_world_stations!(self).recipe_index, &refused);
                taken_back = removed.len();
                if self.server.lan_acting.is_some() && !removed.is_empty() {
                    self.server.guest_packets.push(C::RecipeBookRemove(
                        sunrise_protocol::packets::recipes::RecipeBookRemove { ids: removed },
                    ));
                }
            }
        } else {
            allowed.extend_from_slice(unlocked);
        }
        // Server records unlocked recipes without showing a client toast (clients get ClientboundRecipeBookAddPacket).
        self.common.adv.unlocked.extend_from_slice(&allowed);
        if !allowed.is_empty() {
            self.invalidate_mod_book_placement_knowledge();
        }
        if allowed.is_empty() || self.no_player_here() {
            return taken_back;
        }
        self.load_advancements();
        let tree = self.common.adv.tree.clone();
        for r in &allowed {
            let list = tree.recipe_criteria(r);
            self.trigger_criteria(&tree, &list, |c, w| {
                adv::id_matches(&c["recipes"], "recipe", r, w)
            });
        }
        taken_back
    }
    /// `ServerPlayer.awardRecipes`: fake players learn no recipes.
    pub fn award_recipes(&mut self, recipes: &[String]) -> usize {
        if self.server.lan_acting_fake {
            return 0;
        }
        let index = &server_world_stations!(self).recipe_index;
        let (count, unlocked) = self.common.level.recipe_book.add_recipes(index, recipes);
        count.saturating_sub(self.recipe_unlocked(&unlocked))
    }

    /// `KnowledgeBookItem.use`: the book is used up and its recipes pass through
    /// `award_recipes`; a fake player learns nothing. A book naming a recipe this server does not have is
    /// vanilla's `Invalid recipe: {}` and knows nothing. Whether the click
    /// ends here: `InteractionResult.SUCCESS` does and swings, `FAIL` leaves
    /// the other hand its turn (`Minecraft.startUseItem`).
    pub fn use_knowledge_book(&mut self, stack: &ItemStack) -> bool {
        if &*stack.item != "minecraft:knowledge_book" {
            return false;
        }
        let slot = self.common.held_slot();
        let mut hand = self.common.stack_in(slot).cloned();
        let infinite = self.common.creative;
        let index = &world_stations!(self).recipe_index;
        let result =
            sunrise_sim::recipe_book::Settings::knowledge_book_recipes(index, &mut hand, infinite);
        let depleted = hand
            .is_none()
            .then(|| self.common.stack_in(slot).cloned())
            .flatten();
        *self.common.stack_in_mut(slot) = hand;
        if let Some(original) = depleted {
            self.player_item_destroyed(&original, self.common.item_slot_hand(slot));
        }
        match result {
            Ok(recipes) => {
                self.award_recipes(&recipes);
                self.award_stat("used", &stack.item, 1);
                self.swing_arm();
                true
            }
            Err(invalid) => {
                if let Some(id) = invalid {
                    log::error!("Invalid recipe: {id}");
                }
                false
            }
        }
    }
    /// `PlayerAdvancements.award` once done: `AdvancementRewards.grant`,
    /// then toast and announcement.
    pub(in crate::game::server_side) fn award(&mut self, index: usize) {
        if self.server.lan_acting_fake {
            return;
        }
        let tree = self.common.adv.tree.clone();
        self.award_definition(&tree.list[index]);
    }

    pub(in crate::game) fn award_definition(&mut self, a: &adv::Advancement) {
        if self.server.lan_acting_fake {
            return;
        }
        if let (Some(player), Some(name)) = (
            self.server.lan_acting_uuid,
            self.server.lan_acting_name.clone(),
        ) {
            self.native_player_advancement_done(player, &name, &a.id);
        }
        if self.defer_merchant_reward(a) {
            return;
        }
        self.commit_merchant_reward(a);
    }

    /// Publish a completed advancement's rewards after its tentative criterion commits.
    pub(in crate::game::server_side) fn commit_merchant_reward(&mut self, a: &adv::Advancement) {
        let rewards = self.grant_rewards(a);
        if let Some(d) = &a.display
            && d.announce
            && self
                .common
                .ui
                .rules
                .bool_at(sunrise_game_data::gamerules::vanilla::SHOW_ADVANCEMENT_MESSAGES)
        {
            let key = match d.frame {
                Frame::Task => "chat.type.advancement.task",
                Frame::Goal => "chat.type.advancement.goal",
                Frame::Challenge => "chat.type.advancement.challenge",
            };
            let name = adv_name(a);
            // PlayerList.broadcastSystemMessage: everyone hears of a
            // network player's advancement.
            match self.server.lan_acting_name.clone() {
                // Player.getDisplayName.
                Some(_) => {
                    let player = self.target_name(super::commands::Target::Player);
                    self.common
                        .adv
                        .announced
                        .push(Text::translate(key, &[player, name]));
                }
                None => {
                    let text = Text::translate(key, &[self.player_display_text(), name]);
                    // The guests hear of the host's advancement too.
                    if let Some(host) = &self.server.lan {
                        host.hub
                            .system_message(sunrise_sim::lan::data::component_nbt(
                                &text.component(),
                            ));
                    }
                    if self.server.lan.is_none() {
                        self.server_system_message(text);
                    }
                }
            }
        }
        self.recipe_unlocked(&rewards);
    }
    /// `AdvancementRewards.grant`: the experience, recipes, loot and function of
    /// a finished advancement, and the recipes it unlocked for the
    /// `recipe_unlocked` trigger, which the caller runs after its toast and
    /// announcement. Returns those recipes.
    pub(in crate::game::server_side) fn grant_rewards(
        &mut self,
        a: &adv::Advancement,
    ) -> Vec<String> {
        // AdvancementRewards.grant: giveExperiencePoints runs even for 0.
        self.give_experience_points(a.rewards.experience);
        // ServerPlayer.awardRecipesByKey: the rewards that name a recipe.
        let rewards: Vec<String> = a
            .rewards
            .recipes
            .iter()
            .filter(|r| world_stations!(self).recipe_index.contains(r))
            .cloned()
            .collect();
        let (_, unlocked) = self
            .common
            .level
            .recipe_book
            .add_recipes(&server_world_stations!(self).recipe_index, &rewards);
        // `AdvancementRewards.loot` is a `HolderSet<LootTable>`: ids, or a tag's members in order
        // (a tag that is not bound refused the advancement when the tree loaded).
        let loot = self.server.server_loot.0.get();
        let tables: Vec<String> = a
            .rewards
            .loot
            .iter()
            .flat_map(|entry| match entry.strip_prefix('#') {
                Some(tag) => loot
                    .as_ref()
                    .and_then(|loot| loot.registry_tags("loot_table")?.get(&*adv::full(tag)))
                    .into_iter()
                    .flatten()
                    .cloned()
                    .collect::<Vec<String>>(),
                None => vec![entry.clone()],
            })
            .collect();
        for table in &tables {
            // `LootContextParamSets.ADVANCEMENT_REWARD`: the rewarded player is
            // THIS_ENTITY and their position the ORIGIN.
            let player = self.killer_loot_entity(None);
            let origin = self
                .common
                .player
                .as_ref()
                .map(|p| [p.position.x, p.position.y, p.position.z]);
            let items = self
                .roll_table_at(table, origin, None, Some(&player))
                .unwrap_or_default();
            for s in items {
                // Inventory.add returns false when anything is left, and
                // AdvancementRewards then drops that remainder with no
                // pickup sound. A stack the inventory takes whole plays
                // entity.item.pickup at the player's feet.
                if let Some(rest) = self.add_stack_to_inventory(s) {
                    if let Some(eye) = self.common.eye_position() {
                        self.spawn_item(rest, eye, [0.0; 3], 0);
                    }
                } else if let Some((at, pitch)) = self.common.player.as_mut().map(|p| {
                    let at = [p.position.x, p.position.y, p.position.z];
                    let pitch = ((p.random.next_float() - p.random.next_float()) * 0.7 + 1.0) * 2.0;
                    (at, pitch)
                }) {
                    self.send_level_sound(
                        sunrise_gameplay_common::game::level_sounds_common::LevelSound::new(
                            "minecraft:entity.item.pickup",
                            at,
                            0.2,
                            pitch,
                        )
                        .in_source(mod_api::SoundSource::Players),
                        None,
                    );
                }
            }
        }
        if let Some(function) = &a.rewards.function {
            self.run_advancement_reward_function(function);
        }
        unlocked
    }
    /// `InventoryChangeTrigger`, checked when the inventory changes.
    pub fn check_inventory_trigger(&mut self) {
        use std::hash::{Hash, Hasher};
        // A player in another level than the one in place has their criteria and rewards
        // judged in their own level, from their own tick (`tick_advancements`, which sees the
        // changed inventory as `broadcastChanges` does): judging here would drop a reward in
        // this level.
        if !self.acting_in_level_in_place() {
            return;
        }
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for s in self
            .common
            .inventory
            .iter()
            .chain(self.common.armor.iter())
            .chain(std::iter::once(&self.common.offhand))
        {
            s.as_ref().map(|s| (&s.item, s.count)).hash(&mut h);
        }
        let key = h.finish();
        if key == self.common.adv.last_inventory {
            return;
        }
        self.common.adv.last_inventory = key;
        // Native mods hear the change before any criterion is looked at, so
        // they do with no `inventory_changed` advancement in the tree too
        // (`native_player_inventory_changed`, API 579).
        if !self.no_player_here()
            && mod_api::__private::listened(mod_api::HookId::PlayerInventoryChanged)
        {
            let stacks = inventory_slots(
                &self.common.inventory,
                &self.common.armor,
                &self.common.offhand,
            );
            let player = self.native_acting_id();
            self.native_player_inventory_changed(player, &stacks);
        }
        let stacks: Vec<ItemView> = self
            .common
            .inventory
            .iter()
            .chain(self.common.armor.iter())
            .chain(std::iter::once(&self.common.offhand))
            .flatten()
            .map(item_view)
            .collect();
        // InventoryChangeTrigger slots: occupied, full and empty counts.
        let all: Vec<&Option<ItemStack>> = self
            .common
            .inventory
            .iter()
            .chain(self.common.armor.iter())
            .chain(std::iter::once(&self.common.offhand))
            .collect();
        let occupied = all.iter().filter(|s| s.is_some()).count() as f64;
        let empty = all.len() as f64 - occupied;
        let full = all
            .iter()
            .filter_map(|s| s.as_ref())
            .filter(|s| {
                self.common
                    .res
                    .items()
                    .get(&s.item)
                    .is_some_and(|i| s.count >= i.max_stack)
            })
            .count() as f64;
        // Only the criteria the items held could pass (`InventoryIndex`).
        if self.no_player_here() {
            return;
        }
        self.load_advancements();
        let tree = self.common.adv.tree.clone();
        let index = self.inventory_index();
        let list = index.candidates(stacks.iter().map(|s| s.id.as_str()));
        if list.is_empty() {
            return;
        }
        let me = self.player_entity_for("minecraft:inventory_changed");
        self.trigger_criteria(&tree, &list, move |c, w| {
            let slots = &c["slots"];
            let range = |k: &str, v: f64| {
                let r = &slots[k];
                r.is_null()
                    || r.as_f64().is_some_and(|n| (n - v).abs() < 0.5)
                    || r.is_object()
                        && r["min"].as_f64().is_none_or(|m| v >= m)
                        && r["max"].as_f64().is_none_or(|m| v <= m)
            };
            if !(range("occupied", occupied) && range("full", full) && range("empty", empty)) {
                return false;
            }
            let mut wanted: Vec<&Value> = c["items"]
                .as_array()
                .map(|a| a.iter().collect())
                .unwrap_or_default();
            for s in &stacks {
                wanted.retain(|p| !adv::item(p, s, w));
            }
            // `InventoryChangeTrigger.TriggerInstance`'s `player`
            // (`ContextAwarePredicate`), tested by `SimpleCriterionTrigger.trigger`.
            wanted.is_empty()
                && adv::conditions(
                    &c["player"],
                    &adv::Entities {
                        this: Some(&me),
                        origin: me.position,
                        tool: None,
                        event_block: None,
                        target: None,
                        interacting: None,
                    },
                    w,
                )
        });
    }
    /// Per-tick checks for the player already in place, without a guest connection turn.
    pub fn tick_advancements(&mut self) {
        self.tick_level_advancements();
        if let Some(player) = &self.common.player
            && player.health.tick_count % 20 == 0
        {
            self.trigger_location_advancement();
        }
    }

    /// Criteria on the level path (`ServerPlayer.tick`).
    pub(in crate::game::server_side) fn tick_level_advancements(&mut self) {
        if self.common.player.is_none() {
            return;
        }
        self.check_inventory_trigger();
        // ServerPlayer.tick: PlayerTrigger.TICK every tick.
        let me = self.player_entity_for("minecraft:tick");
        self.trigger("minecraft:tick", |c, w| {
            let ctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                event_block: None,
                target: None,
                interacting: None,
            };
            adv::conditions(&c["player"], &ctx, w)
        });
        // ServerPlayer.onEffectAdded and onEffectsRemoved keep the start of a
        // levitation; tick fires LEVITATION while there is one.
        let levitating = self
            .common
            .potions
            .effects
            .list
            .iter()
            .any(|e| e.id == "levitation");
        match (levitating, self.common.adv.levitation) {
            (true, None) => {
                let at = self
                    .common
                    .player
                    .as_ref()
                    .map(|p| [p.position.x, p.position.y, p.position.z]);
                self.common.adv.levitation = at.map(|at| (at, self.common.ticks));
            }
            (false, Some(_)) => self.common.adv.levitation = None,
            _ => {}
        }
        if let Some((start, since)) = self.common.adv.levitation {
            self.trigger_levitation(start, (self.common.ticks - since) as i64);
        }
        self.tick_marks();
    }

    /// `ServerPlayer.doTick` fires LOCATION after the player body, every 20 entity ticks.
    pub(in crate::game) fn trigger_location_advancement(&mut self) {
        let me = self.player_entity_for("minecraft:location");
        self.trigger("minecraft:location", |c, w| {
            let ctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                event_block: None,
                target: None,
                interacting: None,
            };
            adv::conditions(&c["player"], &ctx, w)
        });
    }
    /// A player condition plus one entity (`player`, `entity` keys).
    pub(in crate::game::server_side) fn trigger_entity(
        &mut self,
        name: &str,
        key: &str,
        target: &EntityView,
    ) {
        let me = self.player_entity_for(name);
        let target = target.clone();
        let key = key.to_owned();
        self.trigger(name, move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                event_block: None,
                target: None,
                interacting: None,
            };
            let ectx = adv::Entities {
                this: Some(&target),
                origin: me.position,
                tool: None,
                event_block: None,
                target: None,
                interacting: None,
            };
            adv::conditions(&c["player"], &pctx, w) && adv::conditions(&c[&key], &ectx, w)
        });
    }
    /// `KilledTrigger` (`player_killed_entity`): the entity killed and the
    /// killing blow's damage source (none: any `killing_blow` fails).
    pub(in crate::game) fn trigger_killed(
        &mut self,
        target: &EntityView,
        blow: Option<&adv::DamageView>,
    ) {
        self.trigger_killed_as("minecraft:player_killed_entity", target, blow);
    }
    /// `KilledTrigger.trigger` for any of its three instances
    /// (`player_killed_entity`, `entity_killed_player`,
    /// `kill_mob_near_sculk_catalyst`).
    pub(in crate::game) fn trigger_killed_as(
        &mut self,
        name: &str,
        target: &EntityView,
        blow: Option<&adv::DamageView>,
    ) {
        let me = self.player_entity_for(name);
        let target = target.clone();
        let blow = blow.cloned();
        self.trigger(name, move |c, w| {
            adv::killed(c, &me, &target, blow.as_ref(), w)
        });
    }
    /// `KilledByArrowTrigger`: what the arrow killed and the weapon it came from.
    pub(in crate::game::server_side) fn trigger_killed_by_arrow(
        &mut self,
        victims: &[EntityView],
        weapon: Option<&ItemStack>,
    ) {
        let me = self.player_entity_for("minecraft:killed_by_arrow");
        let victims = victims.to_vec();
        let weapon = weapon.map(item_view);
        self.trigger("minecraft:killed_by_arrow", move |c, w| {
            adv::killed_by_arrow(c, &me, &victims, weapon.as_ref(), w)
        });
    }
    /// `LevitationTrigger`: how far the player rose from where it started
    /// levitating, and for how long.
    pub(in crate::game::server_side) fn trigger_levitation(
        &mut self,
        start: [f64; 3],
        duration: i64,
    ) {
        let me = self.player_entity_for("minecraft:levitation");
        self.trigger("minecraft:levitation", move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                event_block: None,
                target: None,
                interacting: None,
            };
            adv::conditions(&c["player"], &pctx, w)
                && (c["distance"].is_null() || adv::distance(&c["distance"], start, me.position))
                && adv::range_i(&c["duration"], duration)
        });
    }
    /// `ServerPlayer.tick`: `trackStartFallingPosition` and the end of a fall
    /// (`resetFallDistance`), `trackEnteredOrExitedLavaOnVehicle`, the way back
    /// from the Nether, and `onEffectAdded`, `onEffectUpdated` and
    /// `onEffectsRemoved`, which are seen as each tick ends.
    pub(in crate::game::server_side) fn tick_marks(&mut self) {
        let Some(p) = self.common.player.as_ref() else {
            return;
        };
        let at = [p.position.x, p.position.y, p.position.z];
        let falling = p.fall_distance > 0.0;
        let dead = p.health.dead_or_dying();
        let lava = self
            .common
            .riding
            .and_then(|id| self.common.mobs.iter().find(|m| m.id == id))
            .is_some_and(|m| m.fluids.in_lava);
        match (falling, self.common.adv.marks.fall) {
            (true, None) => {
                self.common.adv.marks.fall = Some(at);
                // `trackStartFallingPosition`: the last blast is spent by the
                // fall (whatever made it), and counts only if the player was
                // above where it hit.
                if let Some((impact, cause)) = self.common.adv.marks.blast.take()
                    && impact[1] <= at[1]
                {
                    self.trigger_fall_after_explosion(impact, cause);
                }
            }
            // `resetFallDistance`: a fall ends by landing, climbing or water.
            (false, Some(start)) => {
                self.common.adv.marks.fall = None;
                if !dead {
                    self.trigger_distance("minecraft:fall_from_height", start);
                }
            }
            _ => {}
        }
        match (lava, self.common.adv.marks.lava_ride) {
            (true, None) => self.common.adv.marks.lava_ride = Some(at),
            (true, Some(start)) => self.trigger_distance("minecraft:ride_entity_in_lava", start),
            (false, Some(_)) => self.common.adv.marks.lava_ride = None,
            (false, None) => {}
        }
        // Back in the Overworld from the Nether: measured from where it was entered.
        if let Some(start) = self.common.adv.marks.nether
            && self.common.dim == Dim::Overworld
        {
            self.common.adv.marks.nether = None;
            self.trigger_distance("minecraft:nether_travel", start);
        }
        let effects = self.effects_seen();
        // `onEffectAdded`, `onEffectUpdated` (an `update` that changed the
        // duration, amplifier or flags too) and `onEffectsRemoved`.
        let changes = self.common.potions.effects.changes;
        if effects != self.common.adv.marks.effects
            || changes != self.common.adv.marks.effect_changes
        {
            self.common.adv.marks.effects = effects;
            self.common.adv.marks.effect_changes = changes;
            self.trigger_effects_changed(None);
        }
    }
    /// The (effect, amplifier) pairs `marks.effects` compares.
    pub(in crate::game::server_side) fn effects_seen(&self) -> Vec<(&'static str, i32)> {
        self.common
            .potions
            .effects
            .list
            .iter()
            .map(|e| (e.id, e.amplifier))
            .collect()
    }
    /// `EffectsChangedTrigger` for a change the caller knows the source of;
    /// the end-of-tick check then does not fire it a second time.
    pub(in crate::game) fn trigger_effects_changed_by(&mut self, source: Option<EntityView>) {
        self.common.adv.marks.effects = self.effects_seen();
        self.common.adv.marks.effect_changes = self.common.potions.effects.changes;
        self.trigger_effects_changed(source);
    }
    /// `DistanceTrigger`: the player's `player` condition, `start_position`
    /// at `start`, and the `distance` from `start` to the player.
    pub(in crate::game::server_side) fn trigger_distance(&mut self, name: &str, start: [f64; 3]) {
        self.trigger_distance_at(name, start, None);
    }

    /// Complete a distance trigger at the position recorded by its call site.
    pub(in crate::game::server_side) fn trigger_distance_at(
        &mut self,
        name: &str,
        start: [f64; 3],
        end: Option<[f64; 3]>,
    ) {
        let mut me = self.player_entity_for(name);
        if let Some(end) = end {
            me.position = end;
        }
        self.trigger(name, move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                event_block: None,
                target: None,
                interacting: None,
            };
            adv::conditions(&c["player"], &pctx, w)
                && adv::location(&c["start_position"], start, w)
                && (c["distance"].is_null() || adv::distance(&c["distance"], start, me.position))
        });
    }
    /// `FallAfterExplosionTrigger.trigger`: a blast that hit the player at
    /// `impact` (made by `cause`, or by nothing), measured to where the
    /// player is now. `matches` tests `cause` only when the instance has one,
    /// and a missing cause fails it.
    pub(in crate::game::server_side) fn trigger_fall_after_explosion(
        &mut self,
        impact: [f64; 3],
        cause: Option<EntityView>,
    ) {
        let me = self.player_entity_for("minecraft:fall_after_explosion");
        self.trigger("minecraft:fall_after_explosion", move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                event_block: None,
                target: None,
                interacting: None,
            };
            adv::conditions(&c["player"], &pctx, w)
                && adv::location(&c["start_position"], impact, w)
                && (c["distance"].is_null() || adv::distance(&c["distance"], impact, me.position))
                && (c["cause"].is_null()
                    || cause.as_ref().is_some_and(|cause| {
                        let cctx = adv::Entities {
                            this: Some(cause),
                            origin: me.position,
                            tool: None,
                            event_block: None,
                            target: None,
                            interacting: None,
                        };
                        adv::conditions(&c["cause"], &cctx, w)
                    }))
        });
    }
    /// `EffectsChangedTrigger`: the player's `effects`, and for an effect that
    /// `source` gave, that entity (`source`).
    pub(in crate::game::server_side) fn trigger_effects_changed(
        &mut self,
        source: Option<EntityView>,
    ) {
        let me = self.player_entity_for("minecraft:effects_changed");
        self.trigger("minecraft:effects_changed", move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                event_block: None,
                target: None,
                interacting: None,
            };
            let effects = c["effects"].is_null()
                || adv::entity(
                    &serde_json::json!({ "minecraft:effects": c["effects"] }),
                    &me,
                    w,
                    me.position,
                );
            let source = c["source"].is_null()
                || source.as_ref().is_some_and(|s| {
                    let sctx = adv::Entities {
                        this: Some(s),
                        origin: me.position,
                        tool: None,
                        event_block: None,
                        target: None,
                        interacting: None,
                    };
                    adv::conditions(&c["source"], &sctx, w)
                });
            adv::conditions(&c["player"], &pctx, w) && effects && source
        });
    }
    /// `ItemDurabilityTrigger.trigger` and `TriggerInstance.matches`: a player's
    /// item takes damage (`ItemStack.applyDamage`), `stack` as it is before the
    /// damage and `new_damage` the unclamped damage value it is about to get.
    /// `durability` tests `maxDamage - newDamage`, `delta` tests
    /// `damageValue - newDamage`.
    pub(in crate::game) fn trigger_item_durability(&mut self, stack: &ItemStack, new_damage: i32) {
        if self.no_player_here() {
            return;
        }
        let me = self.player_entity_for("minecraft:item_durability_changed");
        let item = item_view(stack);
        let max = match stack
            .data()
            .and_then(|data| data.other("minecraft:max_damage"))
        {
            Some(sunrise_core::nbt::Tag::Int(value)) => i64::from(*value),
            _ => i64::from(
                self.common
                    .res
                    .items()
                    .get(&stack.item)
                    .map_or(0, |i| i.max_damage),
            ),
        };
        // ItemStack.getDamageValue clamps to 0..=getMaxDamage.
        let damage = i64::from(stack.damage).clamp(0, max);
        let new_damage = i64::from(new_damage);
        self.trigger("minecraft:item_durability_changed", move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                event_block: None,
                target: None,
                interacting: None,
            };
            adv::conditions(&c["player"], &pctx, w)
                && (c["item"].is_null() || adv::item(&c["item"], &item, w))
                && adv::range_i(&c["durability"], max - new_damage)
                && adv::range_i(&c["delta"], damage - new_damage)
        });
    }
    /// `ServerPlayer.updateUsingItem`: USING_ITEM for the item in use.
    pub(in crate::game::server_side) fn trigger_using_item(&mut self, stack: &ItemStack) {
        if self.no_player_here() {
            return;
        }
        let me = self.player_entity_for("minecraft:using_item");
        let it = item_view(stack);
        self.trigger("minecraft:using_item", move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                event_block: None,
                target: None,
                interacting: None,
            };
            adv::conditions(&c["player"], &pctx, w) && adv::item(&c["item"], &it, w)
        });
    }
    /// `UsedEnderEyeTrigger.trigger`: the player threw an eye of ender toward
    /// a structure `sqr_distance` (squared, horizontal) away.
    pub(in crate::game::server_side) fn trigger_used_ender_eye(&mut self, sqr_distance: f64) {
        if self.no_player_here() {
            return;
        }
        let me = self.player_entity_for("minecraft:used_ender_eye");
        self.trigger("minecraft:used_ender_eye", move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                event_block: None,
                target: None,
                interacting: None,
            };
            adv::conditions(&c["player"], &pctx, w) && matches_sqr(&c["distance"], sqr_distance)
        });
    }
    /// The entity a 100 block ray from the eyes meets first, if no block is
    /// in the way (`ProjectileUtil.getEntityHitResult` over every entity but
    /// the player that is not a spectator, then `hasLineOfSight`).
    pub(in crate::game::server_side) fn looking_at_entity(
        &self,
        eye: [f64; 3],
        dir: [f64; 3],
    ) -> Option<EntityView> {
        use super::commands::Target;
        use super::entity::{Entity as _, entity_lists_of, for_each_entity};
        use sunrise_core::geometry::Aabb;
        let mut best: Option<(Target, f64)> = None;
        let mut offer = |t: Option<Target>, b: Aabb| {
            if let Some(t) = t
                && let Some(d) = super::ray_box(eye, dir, 100.0, b)
                && best.is_none_or(|(_, bd)| d < bd)
            {
                best = Some((t, d));
            }
        };
        for_each_entity!(entity_lists_of!(self), e => {
            if !e.is_removed() {
                offer(e.selector_target(), e.bounds());
            }
        });
        let aabb = |(lo, hi): ([f64; 3], [f64; 3])| {
            Aabb::new(
                Vec3::new(lo[0], lo[1], lo[2]),
                Vec3::new(hi[0], hi[1], hi[2]),
            )
        };
        for f in self.common.item_frames.iter() {
            offer(Some(Target::Frame(f.id)), aabb(f.bounds()));
        }
        for p in self.common.paintings.iter() {
            offer(Some(Target::Painting(p.id)), aabb(p.bounds()));
        }
        for c in self.common.crystals.iter() {
            offer(Some(Target::Crystal(c.id)), c.bounds());
        }
        let (t, _) = best?;
        // `LivingEntity.hasLineOfSight`: eye to the target's eye height,
        // `ClipContext.Block.COLLIDER`, no fluids, within 128 blocks.
        let at = self.target_position(t)?;
        let from = Vec3::new(eye[0], eye[1], eye[2]);
        let to = Vec3::new(at[0], at[1] + self.server_target_eye_height(t), at[2]);
        let world = self.mob_world_base(&[]);
        let clear = to.distance(from) <= 128.0 && world.clear_between(from, to, false);
        if !clear {
            return None;
        }
        Some(match t {
            Target::Mob(id) => self.mob_entity(self.common.mobs.iter().find(|m| m.id == id)?),
            Target::Boat(id) => self.boat_entity(self.common.boats.iter().find(|b| b.id == id)?),
            other => self.entity_view_of(other),
        })
    }
    /// `PlayerHurtEntityTrigger`: the player, the damage and the victim.
    pub(in crate::game::server_side) fn trigger_player_hurt_entity(
        &mut self,
        victim: &EntityView,
        damage: &adv::DamageView,
    ) {
        let me = self.player_entity_for("minecraft:player_hurt_entity");
        let (victim, damage) = (victim.clone(), damage.clone());
        self.trigger("minecraft:player_hurt_entity", move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                event_block: None,
                target: None,
                interacting: None,
            };
            let vctx = adv::Entities {
                this: Some(&victim),
                ..pctx
            };
            adv::conditions(&c["player"], &pctx, w)
                && adv::damage(&c["damage"], &damage, w, me.position)
                && adv::conditions(&c["entity"], &vctx, w)
        });
    }
    /// The player's hit on `victim` as `LivingEntity.hurtServer` reports it:
    /// `kind` damage of `amount` from `direct` (the player's own hand when
    /// `None`), nothing blocked.
    pub(in crate::game) fn player_hit(
        &mut self,
        victim: &EntityView,
        kind: &str,
        direct: Option<EntityView>,
        amount: f32,
    ) {
        let me = self.player_entity_for("minecraft:player_hurt_entity");
        self.trigger_player_hurt_entity(
            victim,
            &adv::DamageView {
                kind: kind.to_owned(),
                is_direct: direct.is_none(),
                direct: direct.or_else(|| Some(me.clone())),
                source: Some(me),
                original: amount,
                actual: amount,
                blocked: false,
            },
        );
    }
    /// `EntityHurtPlayerTrigger`: the player and the damage it took.
    pub fn trigger_entity_hurt_player(&mut self, damage: &adv::DamageView) {
        let me = self.player_entity_for("minecraft:entity_hurt_player");
        let damage = damage.clone();
        self.trigger("minecraft:entity_hurt_player", move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                event_block: None,
                target: None,
                interacting: None,
            };
            adv::conditions(&c["player"], &pctx, w)
                && adv::damage(&c["damage"], &damage, w, me.position)
        });
    }
    /// A player condition plus one item (`item` key).
    pub(in crate::game) fn trigger_item(&mut self, name: &str, stack: &ItemStack) {
        let me = self.player_entity_for(name);
        let it = item_view(stack);
        self.trigger(name, move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                event_block: None,
                target: None,
                interacting: None,
            };
            adv::conditions(&c["player"], &pctx, w) && adv::item(&c["item"], &it, w)
        });
    }
    pub(in crate::game) fn trigger_item_entity(
        &mut self,
        name: &str,
        stack: &ItemStack,
        target: &EntityView,
    ) {
        let me = self.player_entity_for(name);
        let it = item_view(stack);
        self.trigger(name, |c, w| adv::item_entity(c, &me, target, &it, w));
    }
    /// `ServerPlayer.onItemPickup`: the player in place (the host, or the
    /// guest `as_guest` runs) picked up an item entity whose owner
    /// (`ItemEntity.getOwner`, the thrower) is `thrower`: the whole stack, or
    /// what is left after a partial take. An owner that is that player also
    /// fires `LivingEntity.onItemPickup` (the by-entity trigger); one that is
    /// gone, or another player, fires nothing here.
    pub(in crate::game) fn trigger_thrown_picked_up_by_player(
        &mut self,
        stack: &ItemStack,
        thrower: [i32; 4],
    ) {
        let target = if thrower == self.acting_uuid() {
            // The player is the entity of both triggers.
            let me = self.player_entity_for_any(&[
                "minecraft:thrown_item_picked_up_by_entity",
                "minecraft:thrown_item_picked_up_by_player",
            ]);
            self.trigger_item_entity("minecraft:thrown_item_picked_up_by_entity", stack, &me);
            me
        } else if let Some(id) = self.mob_by_uuid(thrower) {
            let Some(m) = self.common.mobs.iter().find(|m| m.id == id) else {
                return;
            };
            self.mob_entity(m)
        } else {
            return;
        };
        self.trigger_item_entity("minecraft:thrown_item_picked_up_by_player", stack, &target);
    }
    /// `ConstructBeaconTrigger`: the pyramid levels of the new beacon.
    pub(in crate::game) fn trigger_construct_beacon(&mut self, levels: i32) {
        let me = self.player_entity_for("minecraft:construct_beacon");
        self.trigger("minecraft:construct_beacon", move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                event_block: None,
                target: None,
                interacting: None,
            };
            adv::conditions(&c["player"], &pctx, w) && adv::range_i(&c["level"], i64::from(levels))
        });
    }
    /// `EnchantedItemTrigger`: the enchanted item and the levels it cost.
    pub(in crate::game) fn trigger_enchanted_item(&mut self, stack: &ItemStack, levels: i32) {
        let me = self.player_entity_for("minecraft:enchanted_item");
        let it = item_view(stack);
        self.trigger("minecraft:enchanted_item", move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                event_block: None,
                target: None,
                interacting: None,
            };
            adv::conditions(&c["player"], &pctx, w)
                && adv::item(&c["item"], &it, w)
                && adv::range_i(&c["levels"], i64::from(levels))
        });
    }
    /// `BrewedPotionTrigger`: the potion contents of a stack taken from a
    /// brewing stand.
    pub(in crate::game) fn trigger_brewed_potion(&mut self, stack: &ItemStack) {
        let me = self.player_entity_for("minecraft:brewed_potion");
        let potion = stack.potion().map(str::to_owned);
        self.trigger("minecraft:brewed_potion", move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                event_block: None,
                target: None,
                interacting: None,
            };
            adv::conditions(&c["player"], &pctx, w)
                && adv::potions(&c["potion"], potion.as_deref(), w)
        });
    }
    /// `TradeTrigger`: the villager and the item the trade gave.
    pub(in crate::game::server_side) fn trigger_trade(
        &mut self,
        villager: &EntityView,
        stack: &ItemStack,
    ) {
        let me = self.player_entity_for("minecraft:villager_trade");
        let villager = villager.clone();
        let it = item_view(stack);
        self.trigger("minecraft:villager_trade", move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                event_block: None,
                target: None,
                interacting: None,
            };
            let vctx = adv::Entities {
                this: Some(&villager),
                ..pctx
            };
            adv::conditions(&c["player"], &pctx, w)
                && adv::conditions(&c["villager"], &vctx, w)
                && adv::item(&c["item"], &it, w)
        });
    }
    /// `FishingRodHookedTrigger`: the rod, what the hook is in (or the hook)
    /// and the items it caught.
    pub(in crate::game::server_side) fn trigger_fishing_rod_hooked(
        &mut self,
        rod: &ItemStack,
        hooked: &EntityView,
        items: &[ItemStack],
    ) {
        let me = self.player_entity_for("minecraft:fishing_rod_hooked");
        let rod = item_view(rod);
        let hooked = hooked.clone();
        let items: Vec<ItemView> = items.iter().map(item_view).collect();
        self.trigger("minecraft:fishing_rod_hooked", move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                event_block: None,
                target: None,
                interacting: None,
            };
            let ectx = adv::Entities {
                this: Some(&hooked),
                ..pctx
            };
            adv::conditions(&c["player"], &pctx, w)
                && adv::item(&c["rod"], &rod, w)
                && adv::conditions(&c["entity"], &ectx, w)
                && (c["item"].is_null() || items.iter().any(|i| adv::item(&c["item"], i, w)))
        });
    }
    /// `EnterBlockTrigger`: the block, its state properties and the player.
    pub(in crate::game::server_side) fn trigger_enter_block(
        &mut self,
        state: sunrise_world::registry::BlockStateId,
    ) {
        let Some(definition) = self.common.pack.assets.registry.state(state).cloned() else {
            return;
        };
        let me = self.player_entity_for("minecraft:enter_block");
        self.trigger("minecraft:enter_block", move |c, w| {
            let block_ok = adv::id_matches(&c["blocks"], "block", &definition.name, w);
            let state_ok = c["state"]
                .as_object()
                .into_iter()
                .flatten()
                .all(|(k, v)| definition.properties.get(k).map(String::as_str) == v.as_str());
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                event_block: None,
                target: None,
                interacting: None,
            };
            block_ok && state_ok && adv::conditions(&c["player"], &pctx, w)
        });
    }
    /// `SlideDownBlockTrigger`: match the actual block and its state.
    pub(in crate::game::server_side) fn trigger_honey_slide(&mut self, pos: [i32; 3]) {
        let me = self.player_entity_for("minecraft:slide_down_block");
        self.trigger("minecraft:slide_down_block", move |c, w| {
            let ctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                event_block: None,
                target: None,
                interacting: None,
            };
            adv::conditions(&c["player"], &ctx, w) && adv::block(c, pos, w)
        });
    }
    /// `TargetBlockTrigger`: signal and projectile conditions are evaluated in
    /// the shooter's context, including distance from the shooter.
    pub(in crate::game::server_side) fn trigger_target_hit(
        &mut self,
        projectile: &EntityView,
        strength: i32,
    ) {
        let me = self.player_entity_for("minecraft:target_hit");
        let projectile = projectile.clone();
        self.trigger("minecraft:target_hit", move |c, w| {
            let player = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                event_block: None,
                target: None,
                interacting: None,
            };
            let target = adv::Entities {
                this: Some(&projectile),
                ..player
            };
            adv::conditions(&c["player"], &player, w)
                && adv::range_i(&c["signal_strength"], i64::from(strength))
                && adv::conditions(&c["projectile"], &target, w)
        });
    }
    /// `ItemUsedOnLocationTrigger` (placed_block, item_used_on_block).
    pub(in crate::game) fn trigger_block(&mut self, name: &str, pos: [i32; 3], stack: &ItemStack) {
        let _ = self.trigger_block_use(name, pos, Some(stack));
    }
    /// `DefaultBlockInteractionTrigger` (default_block_use): the player, and
    /// the block's `BLOCK_USE` context at its centre, with no tool.
    pub(in crate::game::server_side) fn trigger_default_block_use(&mut self, pos: [i32; 3]) {
        let _ = self.trigger_block_use("minecraft:default_block_use", pos, None);
    }
    /// `AnyBlockInteractionTrigger` (any_block_use): the player, and the
    /// block's `ADVANCEMENT_LOCATION` context at its centre with the original
    /// stack and its in-place edits, as documented on `Game::finish_block_use`.
    pub(in crate::game::server_side) fn trigger_any_block_use(
        &mut self,
        pos: [i32; 3],
        stack: &ItemStack,
    ) {
        let _ = self.trigger_block_use("minecraft:any_block_use", pos, Some(stack));
    }
    pub(in crate::game) fn trigger_block_use(
        &mut self,
        name: &str,
        pos: [i32; 3],
        stack: Option<&ItemStack>,
    ) -> Result<(), mod_api::TriggerCriterionError> {
        #[cfg(any(test, feature = "test-support"))]
        block_use_recording::record(name, stack);
        if self.server.lan_acting_fake || self.no_player_here() {
            return Ok(());
        }
        self.load_advancements();
        let tree = self.common.adv.tree.clone();
        let Some(criteria) = tree.by_trigger.get(name) else {
            return Ok(());
        };
        let [x, y, z] = pos;
        let column = (x >> 4, z >> 4);
        // `Level.isInValidBounds` uses `ChunkPos.isValid`, not the world border.
        // `ChunkPyramid.MAX_CHUNK_COORDINATE_VALUE`: packed X/Z minus the safety margin
        // of (32 + the generation FULL step's 12 dependency distances + 1) * 2 chunks.
        const MAX_CHUNK_COORDINATE: i32 = (33_554_431 >> 4) - (32 + 12 + 1) * 2;
        let in_bounds = self.common.world.shape.contains_y(y)
            && column.0.abs().max(column.1.abs()) <= MAX_CHUNK_COORDINATE;
        let registry = self.common.pack.assets.registry;
        let state = if !in_bounds {
            registry.default_state("minecraft:void_air")
        } else if self.load_full_column(column) {
            self.common
                .world
                .chunks
                .get(&column)
                .map(|chunk| chunk.block(x & 15, y, z & 15))
        } else {
            None
        };
        let Some(state) = state.and_then(|state| registry.state(state)) else {
            log::error!(
                "{}: {name} could not read the block at {pos:?}",
                self.common.dim.id()
            );
            return Err(mod_api::TriggerCriterionError::BlockUnavailable);
        };
        let event_block = Some(adv::BlockEvent {
            state,
            // BLOCK_USE doesn't include BLOCK_ENTITY. ADVANCEMENT_LOCATION does.
            block_entity: (in_bounds && stack.is_some()).then_some(pos),
        });
        let me = self.player_entity_for(name);
        let it = stack.map(item_view);
        let centre = pos.map(|c| f64::from(c) + 0.5);
        self.trigger_criteria(&tree, criteria, move |c, w| {
            let player = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                event_block: None,
                target: None,
                interacting: None,
            };
            let location = adv::Entities {
                origin: centre,
                tool: it.as_ref(),
                event_block,
                ..player
            };
            adv::conditions(&c["location"], &location, w)
                && adv::conditions(&c["player"], &player, w)
        });
        Ok(())
    }
    /// `LightningStrikeTrigger`: the bolt, and the mobs around it that it did
    /// not hit (`bystander`), each tested from this player's position.
    pub(in crate::game::server_side) fn trigger_lightning_strike(
        &mut self,
        bolt: &EntityView,
        viewers: &[EntityView],
    ) {
        let me = self.player_entity_for("minecraft:lightning_strike");
        let (bolt, viewers) = (bolt.clone(), viewers.to_vec());
        self.trigger("minecraft:lightning_strike", move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                event_block: None,
                target: None,
                interacting: None,
            };
            let bctx = adv::Entities {
                this: Some(&bolt),
                ..pctx
            };
            adv::conditions(&c["player"], &pctx, w)
                && adv::conditions(&c["lightning"], &bctx, w)
                && (c["bystander"].is_null()
                    || viewers.iter().any(|v| {
                        adv::conditions(
                            &c["bystander"],
                            &adv::Entities {
                                this: Some(v),
                                ..pctx
                            },
                            w,
                        )
                    }))
        });
    }
    /// `ChanneledLightningTrigger`: each victim condition matched by some
    /// mob the bolt hit, from the player who channelled it.
    pub(in crate::game::server_side) fn trigger_channeled_lightning(
        &mut self,
        victims: &[EntityView],
    ) {
        let me = self.player_entity_for("minecraft:channeled_lightning");
        let victims = victims.to_vec();
        self.trigger("minecraft:channeled_lightning", move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                event_block: None,
                target: None,
                interacting: None,
            };
            adv::conditions(&c["player"], &pctx, w)
                && c["victims"].as_array().into_iter().flatten().all(|want| {
                    victims.iter().any(|v| {
                        adv::conditions(
                            want,
                            &adv::Entities {
                                this: Some(v),
                                ..pctx
                            },
                            w,
                        )
                    })
                })
        });
    }
    /// `SpearMobsTrigger`: the living entities the spear stabbed in this use.
    pub(in crate::game::server_side) fn trigger_spear_mobs(&mut self, stabbed: usize) {
        let me = self.player_entity_for("minecraft:spear_mobs");
        self.trigger("minecraft:spear_mobs", move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                event_block: None,
                target: None,
                interacting: None,
            };
            adv::conditions(&c["player"], &pctx, w)
                && c["count"].as_u64().is_none_or(|n| stabbed as u64 >= n)
        });
    }
    /// `BeeNestDestroyedTrigger`: the nest's block state, the tool that broke
    /// it and the bees left inside.
    pub(in crate::game::server_side) fn trigger_bee_nest_destroyed(
        &mut self,
        state: sunrise_world::registry::BlockStateId,
        tool: Option<&ItemStack>,
        bees: i64,
    ) {
        let Some(definition) = self.common.pack.assets.registry.state(state).cloned() else {
            return;
        };
        let me = self.player_entity_for("minecraft:bee_nest_destroyed");
        let tool = tool.map(item_view).unwrap_or_default();
        self.trigger("minecraft:bee_nest_destroyed", move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                event_block: None,
                target: None,
                interacting: None,
            };
            let state_ok = c["state"]
                .as_object()
                .into_iter()
                .flatten()
                .all(|(k, v)| definition.properties.get(k).map(String::as_str) == v.as_str());
            adv::id_matches(&c["blocks"], "block", &definition.name, w)
                && state_ok
                && adv::item(&c["item"], &tool, w)
                && adv::range_i(&c["num_bees_inside"], bees)
                && adv::conditions(&c["player"], &pctx, w)
        });
    }
    /// `Entity.startRiding`'s `sendEventAndTriggers`: `START_RIDING_TRIGGER`
    /// for every `ServerPlayer` in `entityToRide.getIndirectPassengersStream()`.
    pub(in crate::game::server_side) fn trigger_started_riding(&mut self, vehicle: u32) {
        let mounts: Vec<(u32, Option<u32>)> = self
            .common
            .mobs
            .iter()
            .map(|m| (m.id, m.extra.mount))
            .collect();
        let aboard = super::boarding::passenger_vehicles(vehicle, &mounts);
        self.for_each_player(|g| {
            if g.riding.is_some_and(|v| aboard.contains(&v)) {
                g.trigger_player("minecraft:started_riding");
            }
        });
    }
    /// A trigger with only the player condition (slept_in_bed, started_riding).
    pub(in crate::game) fn trigger_player(&mut self, name: &str) {
        let me = self.player_entity_for(name);
        self.trigger(name, move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                event_block: None,
                target: None,
                interacting: None,
            };
            adv::conditions(&c["player"], &pctx, w)
        });
    }
    /// `RecipeCraftedTrigger`: the recipe and the stacks that went into it
    /// (`ServerPlayer.triggerRecipeCrafted`); an empty slot is air.
    pub(in crate::game) fn trigger_recipe_crafted(
        &mut self,
        recipe: &str,
        ingredients: &[Option<ItemStack>],
    ) {
        let me = self.player_entity_for("minecraft:recipe_crafted");
        let used: Vec<ItemView> = ingredients
            .iter()
            .map(|s| {
                s.as_ref().map_or_else(
                    || ItemView {
                        id: "minecraft:air".to_owned(),
                        ..Default::default()
                    },
                    item_view,
                )
            })
            .collect();
        let recipe = recipe.to_owned();
        self.trigger("minecraft:recipe_crafted", move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                event_block: None,
                target: None,
                interacting: None,
            };
            adv::conditions(&c["player"], &pctx, w)
                && adv::id_matches(&c["recipes"], "recipe", &recipe, w)
                && adv::recipe_ingredients(&c["ingredients"], &used, w)
        });
    }
    /// `LootTableTrigger`: a container's loot table unpacked for this player
    /// (`RandomizableContainer.unpackLootTable`).
    pub(in crate::game::server_side) fn trigger_loot_table(&mut self, table: &str) {
        let me = self.player_entity_for("minecraft:player_generates_container_loot");
        let table = table.to_owned();
        self.trigger("minecraft:player_generates_container_loot", move |c, w| {
            let pctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                event_block: None,
                target: None,
                interacting: None,
            };
            adv::conditions(&c["player"], &pctx, w)
                && adv::id_matches(&c["loot_tables"], "loot_table", &table, w)
        });
    }
    /// `BredAnimalsTrigger`.
    pub(in crate::game::server_side) fn trigger_bred(
        &mut self,
        parent: &EntityView,
        partner: &EntityView,
        child: Option<&EntityView>,
    ) {
        let me = self.player_entity_for("minecraft:bred_animals");
        let (parent, partner, child) = (parent.clone(), partner.clone(), child.cloned());
        self.trigger("minecraft:bred_animals", move |c, w| {
            let ctx = |e: &EntityView| {
                adv::Entities {
                    this: Some(e),
                    origin: me.position,
                    tool: None,
                    event_block: None,
                    target: None,
                    interacting: None,
                }
                .this
                .map(|_| ())
            };
            let _ = ctx;
            let test = |key: &str, e: Option<&EntityView>| {
                c[key].is_null()
                    || e.is_some_and(|e| {
                        adv::conditions(
                            &c[key],
                            &adv::Entities {
                                this: Some(e),
                                origin: me.position,
                                tool: None,
                                event_block: None,
                                target: None,
                                interacting: None,
                            },
                            w,
                        )
                    })
            };
            // matches(parent, partner, child) or (partner, parent, child).
            (test("parent", Some(&parent)) && test("partner", Some(&partner))
                || test("parent", Some(&partner)) && test("partner", Some(&parent)))
                && test("child", child.as_ref())
        });
    }
    /// `ChangeDimensionTrigger`, and `ServerPlayer.enteredNetherPosition`: the
    /// Overworld position the player went into the Nether from (`at`) is kept
    /// for the way back, and any other trip drops it.
    pub(in crate::game::server_side) fn trigger_dimension(
        &mut self,
        from: Dim,
        to: Dim,
        at: [f64; 3],
    ) {
        self.trigger_changed_dimension(from, to);
        if from == Dim::Overworld && to == Dim::Nether {
            self.common.adv.marks.nether = Some(at);
        } else if !(from == Dim::Nether && to == Dim::Overworld) {
            self.common.adv.marks.nether = None;
        }
    }
    /// `CriteriaTriggers.CHANGED_DIMENSION` alone, without the
    /// `enteredNetherPosition` bookkeeping of
    /// `ServerPlayer.triggerDimensionChangeTriggers`: a respawn after the
    /// credits triggers the criterion (`END` → `OVERWORLD`, whatever level
    /// the point is in) and leaves that position as it is.
    pub(in crate::game::server_side) fn trigger_changed_dimension(&mut self, from: Dim, to: Dim) {
        let (from_id, to_id) = (from.id().to_owned(), to.id().to_owned());
        self.trigger("minecraft:changed_dimension", move |c, _| {
            c["from"].as_str().is_none_or(|f| adv::full(f) == from_id)
                && c["to"].as_str().is_none_or(|t| adv::full(t) == to_id)
        });
    }
}
/// `Advancement.name`: the bracketed title in the frame's colour, with the
/// description on hover; the id when there is no display.
pub(in crate::game) fn adv_name(a: &adv::Advancement) -> Text {
    let Some(d) = &a.display else {
        return Text::literal(a.id.clone());
    };
    let color = frame_color(d.frame);
    let title = text_of(&d.title).with_style(&Style {
        hover: Some(std::sync::Arc::new(decorate_name(d))),
        ..Style::default()
    });
    Text::translate("chat.square_brackets", &[title]).with_color(color)
}
pub(in crate::game::server_side) fn frame_color(frame: Frame) -> u32 {
    match frame {
        Frame::Challenge => text::DARK_PURPLE,
        _ => text::GREEN,
    }
}
/// `Advancement.decorateName`: a copy of the title with the frame's colour
/// under its own style (mergeStyles), then a new line and the description
/// after the title's own siblings, so both inherit the title's root style.
/// Built on the title's JSON (a list as its first element with the rest as
/// siblings): a parsed Text moves a styled root's style down a level once it
/// has siblings.
pub(in crate::game::server_side) fn decorate_name(d: &adv::Display) -> Text {
    let mut component = match sunrise_game_data::scoreboard::canonical_component(&d.title) {
        Value::String(s) => serde_json::json!({ "text": s }),
        c => c,
    };
    if let Some(o) = component.as_object_mut() {
        let frame = Text::empty().with_color(frame_color(d.frame)).component();
        o.entry("color").or_insert_with(|| frame["color"].clone());
        let extra = o.entry("extra").or_insert_with(|| serde_json::json!([]));
        if let Some(extra) = extra.as_array_mut() {
            extra.extend(["\n".into(), d.description.clone()]);
        }
    }
    text_of(&component).with_component(component)
}
impl crate::game::ServerGame<'_> {
    /// `AdvancementCommands.Action.GRANT` on one advancement (or one of
    /// its criteria): false when nothing changed.
    pub fn grant_advancement(&mut self, index: usize, criterion: Option<&str>) -> bool {
        if self.server.lan_acting_fake {
            return false;
        }
        let tree = self.common.adv.tree.clone();
        let a = &tree.list[index];
        if criterion.is_none() && self.common.adv.progress.done(a) {
            return false;
        }
        let names: Vec<&str> = match criterion {
            Some(c) => vec![c],
            None => a.criteria.iter().map(|c| c.name.as_str()).collect(),
        };
        let date = now_text();
        let mut changed = false;
        for c in names {
            let was = self.common.adv.progress.done(a);
            if self.common.adv.progress.grant(&a.id, c, &date) {
                changed = true;
                self.advancement_progress(a, c, ProgressType::Grant);
                self.progress_changed(index);
                if !was && self.common.adv.progress.done(a) {
                    self.award(index);
                }
            }
        }
        changed || criterion.is_none()
    }
    /// `AdvancementCommands.Action.REVOKE`.
    pub fn revoke_advancement(&mut self, index: usize, criterion: Option<&str>) -> bool {
        if self.server.lan_acting_fake {
            return false;
        }
        let tree = self.common.adv.tree.clone();
        let a = &tree.list[index];
        self.progress_changed(index);
        match criterion {
            Some(c) => self.revoke_criterion(a, c),
            None => {
                let mut any = false;
                for c in &a.criteria {
                    any |= self.revoke_criterion(a, &c.name);
                }
                any
            }
        }
    }
    /// `PlayerAdvancements.revoke(holder, criterion)`: the progress change and
    /// the hook for it.
    pub(in crate::game::server_side) fn revoke_criterion(
        &mut self,
        advancement: &adv::Advancement,
        criterion: &str,
    ) -> bool {
        let revoked = self.common.adv.progress.revoke(&advancement.id, criterion);
        if revoked {
            self.advancement_progress(advancement, criterion, ProgressType::Revoke);
        }
        revoked
    }
}
#[cfg(test)]
pub(in crate::game) mod tests {
    use sunrise_gameplay_common::game::server_side::advancements::tests::announced;

    use super::*;

    #[test]
    fn looking_at_triggers_are_the_ones_that_ask() {
        let criterion = |trigger: &str, conditions: serde_json::Value| adv::Criterion {
            name: "c".into(),
            trigger: trigger.into(),
            conditions: sunrise_world::lazy_json::LazyJson::new(&conditions),
            player: None,
        };
        let looking = serde_json::json!({"player": {"type": "minecraft:entity_properties",
            "entity": "this", "predicate": {"minecraft:type_specific/player":
            {"looking_at": {"minecraft:entity_type": "minecraft:oak_boat"}}}}});
        let reference = serde_json::json!({"player": [{"condition": "minecraft:reference",
            "name": "pack:looks"}]});
        let plain = serde_json::json!({"player": {"type": "minecraft:entity_properties",
            "entity": "this", "predicate": {"minecraft:flags": {"is_sneaking": true}}}});
        let mut tree = Tree::default();
        tree.list.push(adv::Advancement {
            id: "pack:a".into(),
            parent: None,
            display: None,
            criteria: vec![
                criterion("minecraft:using_item", looking),
                criterion("minecraft:location", reference),
                criterion("minecraft:tick", plain),
            ],
            requirements: Vec::new(),
            rewards: Default::default(),
        });
        tree.index_looking_at();
        assert!(tree.may_look("minecraft:using_item"));
        assert!(tree.may_look("minecraft:location"));
        assert!(!tree.may_look("minecraft:tick"));
    }

    #[test]
    fn nbt_triggers_are_the_ones_that_ask() {
        let criterion = |trigger: &str, conditions: serde_json::Value| adv::Criterion {
            name: "c".into(),
            trigger: trigger.into(),
            conditions: sunrise_world::lazy_json::LazyJson::new(&conditions),
            player: None,
        };
        let nbt = serde_json::json!({"entity": [{"condition": "minecraft:entity_properties",
            "entity": "this", "predicate": {"minecraft:nbt": "{Variant:1}"}}]});
        let reference = serde_json::json!({"player": [{"condition": "minecraft:reference",
            "name": "pack:saved"}]});
        let plain = serde_json::json!({"entity": [{"condition": "minecraft:entity_properties",
            "entity": "this", "predicate": {"minecraft:flags": {"is_baby": true}}}]});
        let mut tree = Tree::default();
        tree.index_nbt();
        assert!(!tree.tests_nbt());
        tree.list.push(adv::Advancement {
            id: "pack:a".into(),
            parent: None,
            display: None,
            criteria: vec![
                criterion("minecraft:player_interacted_with_entity", nbt),
                criterion("minecraft:location", reference),
                criterion("minecraft:tame_animal", plain),
            ],
            requirements: Vec::new(),
            rewards: Default::default(),
        });
        tree.index_nbt();
        assert!(tree.tests_nbt());
        assert!(tree.may_test_nbt("minecraft:player_interacted_with_entity"));
        assert!(tree.may_test_nbt("minecraft:location"));
        assert!(!tree.may_test_nbt("minecraft:tame_animal"));
    }

    #[test]
    fn titles_keep_their_siblings_and_styles() {
        let title = text_of(&serde_json::json!({
            "text": "",
            "extra": [{"text": "Quest ", "color": "gold"}, "Board"],
        }));
        assert_eq!(title.plain(), "Quest Board");
        assert_eq!(title.spans[0].style.color, Some(text::GOLD));
        assert_eq!(title.spans[1].style.color, None);
        let vanilla = text_of(&serde_json::json!({"translate": "advancements.story.root.title"}));
        assert_eq!(
            vanilla,
            Text::translate("advancements.story.root.title", &[])
        );
        assert_eq!(
            vanilla.translation_key(),
            Some("advancements.story.root.title")
        );
    }

    /// The chat hover: the JSON a client receives, and the spans drawn here.
    pub(in crate::game::server_side) fn hover(a: &adv::Advancement) -> (Value, Text) {
        let local = decorate_name(a.display.as_ref().unwrap());
        let sent = adv_name(a).component()["with"][0]["hover_event"]["value"].clone();
        (sent, local)
    }

    #[test]
    fn chat_hover_keeps_the_titles_siblings_before_the_description() {
        let a = announced(
            serde_json::json!({"text": "", "extra": [{"text": "Quest ", "color": "gold"}, "Board"]}),
            serde_json::json!("Open the board"),
        );
        let (sent, local) = hover(&a);
        let sent = sunrise_game_data::scoreboard::comp_text(&sent);
        assert_eq!(sent.plain(), "Quest Board\nOpen the board");
        let colors = |t: &Text| t.spans.iter().map(|s| s.style.color).collect::<Vec<_>>();
        let (gold, green) = (Some(text::GOLD), Some(text::GREEN));
        assert_eq!(colors(&sent), [gold, green, green, green]);
        // The spans drawn here come from the same JSON a client receives.
        assert_eq!(colors(&local), colors(&sent));
    }

    #[test]
    fn chat_hover_description_takes_a_styled_titles_own_style() {
        // mergeStyles keeps the title's gold; the description inherits it.
        let object = announced(
            serde_json::json!({"text": "Quest", "color": "gold", "bold": true, "extra": [" Board"]}),
            serde_json::json!("Open the board"),
        );
        let list = announced(
            serde_json::json!([{"text": "Quest", "color": "gold", "bold": true}, " Board"]),
            serde_json::json!("Open the board"),
        );
        for a in [object, list] {
            let (sent, local) = hover(&a);
            let sent = sunrise_game_data::scoreboard::comp_text(&sent);
            assert_eq!(sent.plain(), "Quest Board\nOpen the board");
            let last = &sent.spans.last().unwrap().style;
            assert_eq!((last.color, last.bold), (Some(text::GOLD), Some(true)));
            assert_eq!(local.spans.last().unwrap().style, *last);
        }
    }

    #[test]
    fn chat_hover_of_a_vanilla_advancement() {
        let a = announced(
            serde_json::json!({"translate": "advancements.story.root.title"}),
            serde_json::json!({"translate": "advancements.story.root.description"}),
        );
        assert_eq!(
            hover(&a).0,
            serde_json::json!({
                "translate": "advancements.story.root.title",
                "color": "green",
                "extra": ["\n", {"translate": "advancements.story.root.description"}],
            })
        );
    }
}

/// A player's 41 slots in `Inventory` numbering, for
/// `native_player_inventory_changed`: the main inventory (0..36), the armour
/// feet to head (36..40; `armor` holds it head first), the off hand (40).
pub fn inventory_slots(
    inventory: &[Option<ItemStack>],
    armor: &[Option<ItemStack>; 4],
    offhand: &Option<ItemStack>,
) -> Vec<Option<ItemStack>> {
    let mut slots: Vec<Option<ItemStack>> = inventory.iter().take(36).cloned().collect();
    slots.resize(36, None);
    slots.extend(armor.iter().rev().cloned());
    slots.push(offhand.clone());
    slots
}

use sunrise_gameplay_common::game::side_ops::presentation::PresentationSideGameExt as _;

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::weather::WeatherGameExt as _;

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::item_use_stacks::ItemUseStacksGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::ModGameExt1 as _;

use sunrise_gameplay_common::game::scoreboard_common::CommonGameExt1 as _;

#[cfg(any(test, feature = "test-support"))]
pub mod block_use_recording;
