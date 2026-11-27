//! Profession preparation participates in the existing WorldTables root publication.
use super::*;
use mod_api::__private::game::RegistrationContextNew;
impl crate::game::ServerGame<'_> {
    pub(in crate::game::server_side) fn prepare_professions(
        &self,
        root: &std::path::Path,
        items: &Items,
        overlay: &sunrise_world::loot::PackOverlay<'_>,
        stations: &mut sunrise_sim::stations::StationData,
    ) -> Result<()> {
        let declarations = self.common.mods.profession_declarations.get_or_init(|| {
            let mut registrar = mod_api::ProfessionDeclarations::default();
            sunrise_gameplay_common::game::native_hooks::native_register_villager_professions(
                &mod_api::RegistrationContext::new(),
                &mut registrar,
            );
            registrar.into_entries()
        });
        let mut poi_types = Vec::new();
        for (owner, definition) in declarations {
            for workstation in &definition.workstations {
                let name = workstation.block.to_string();
                if self
                    .common
                    .pack
                    .assets
                    .registry
                    .default_state(&name)
                    .is_none()
                {
                    return Err(format!(
                        "mod {owner}, profession {}: unknown workstation block {name}",
                        definition.key.0
                    )
                    .into());
                }
                let states: Vec<_> = (0..self.common.pack.assets.registry.len())
                    .filter_map(|index| {
                        let state = self
                            .common
                            .pack
                            .assets
                            .registry
                            .state(sunrise_world::registry::BlockStateId(index as u32))?;
                        (state.name == name
                            && workstation
                                .properties
                                .iter()
                                .all(|(key, value)| state.properties.get(key) == Some(value)))
                        .then(|| state.clone())
                    })
                    .collect();
                if states.is_empty() {
                    return Err(format!("mod {owner}, profession {}: workstation {name} selects no valid block states", definition.key.0).into());
                }
                poi_types.push((workstation.poi.0.to_string(), states));
            }
        }
        let pois = sunrise_world::poi::PoiSnapshot::prepare(poi_types).map_err(|reason| {
            let owners = declarations
                .iter()
                .map(|(owner, definition)| format!("mod {owner}, profession {}", definition.key.0))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{owners}: {reason}")
        })?;
        let mut candidate =
            sunrise_sim::mobs::villager::ResolvedProfessions::prepare(
                declarations.clone(),
                |kind, id| {
                    let name = id.to_string();
                    match kind {
                        "poi" => pois.resolve(&name).is_some(),
                        "trade_set" => stations.trades.sets.contains_key(&name),
                        "item" => items.get(&name).is_some(),
                        "block" => self
                            .common
                            .pack
                            .assets
                            .registry
                            .default_state(&name)
                            .is_some(),
                        "sound" => std::fs::read(
                            root.join("assets").join(id.namespace()).join("sounds.json"),
                        )
                        .ok()
                        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
                        .is_some_and(|events| events.get(id.path()).is_some()),
                        _ => false,
                    }
                },
            )
            .map_err(|errors| {
                format!(
                    "villager professions refused: {}",
                    errors
                        .iter()
                        .map(|error| profession_refusal(error, declarations))
                        .collect::<Vec<_>>()
                        .join("; ")
                )
            })?;
        if !declarations.is_empty() {
            candidate.workstation_pois = (0..self.common.pack.assets.registry.len())
                .map(|index| {
                    self.common
                        .pack
                        .assets
                        .registry
                        .state(sunrise_world::registry::BlockStateId(index as u32))
                        .and_then(|state| pois.custom(state))
                })
                .collect();
        }
        candidate.poi_tags =
            sunrise_world::tag_layers::load(root, "point_of_interest_type", overlay, |id| {
                pois.resolve(id).is_some()
            })?
            .into_iter()
            .map(|(tag, ids)| (tag, ids.into_iter().collect()))
            .collect();
        candidate.pois = pois;
        stations.professions = Arc::new(candidate);
        stations.trades.professions = stations.professions.clone();
        Ok(())
    }
}

impl crate::game::ServerGame<'_> {
    pub(in crate::game::server_side) fn resolve_loaded_profession(
        &mut self,
        mob: &mut sunrise_sim::mobs::Mob,
    ) {
        self.stations().professions.resolve_loaded(mob);
        if let sunrise_sim::mobs::villager::ProfessionState::Unresolved(id) = &mob.extra.profession
            && self.server.missing_professions.insert(id.clone())
        {
            log::warn!(
                "Unknown villager profession {id} at {:?}; preserving its saved identity",
                mob.position
            );
        }
    }
}

impl crate::game::ServerGame<'_> {
    pub(in crate::game::server_side) fn install_profession_save_snapshots(&self) {
        let table = self.stations().professions.clone();
        self.server
            .save
            .0
            .install_profession_snapshot(table.clone());
        self.server
            .levels
            .0
            .install_profession_save_snapshots(table);
    }
}

impl crate::game::ServerGame<'_> {
    pub(in crate::game::server_side) fn with_profession_save_snapshot(
        &self,
        save: Arc<SaveDir>,
    ) -> Arc<SaveDir> {
        save.install_profession_snapshot(self.stations().professions.clone());
        save
    }
}

fn profession_refusal(
    error: &mod_api::ProfessionRegistrationError,
    declarations: &[(String, mod_api::ProfessionDefinition)],
) -> String {
    use mod_api::ProfessionRegistrationError as E;
    let contexts = declarations
        .iter()
        .filter(|(_, definition)| match error {
            E::DuplicateProfession { key, .. }
            | E::ReservedProfession { key }
            | E::InvalidTradeLevel { key, .. } => definition.key == *key,
            E::InvalidWorkstation { key, .. } | E::UnacquirablePoi { key } => definition
                .workstations
                .iter()
                .any(|station| station.poi == *key),
            E::OverlappingPoi { professions, .. } => professions.contains(&definition.key),
            E::UnknownPoi { key } => {
                definition.held_poi.contains(key) || definition.acquirable_poi.contains(key)
            }
            E::UnknownTradeSet { key } => definition.trades.iter().any(|(_, trade)| trade == key),
            E::UnknownItem { key } => definition.requested_items.contains(key),
            E::UnknownBlock { key } => definition.secondary_blocks.contains(key),
            E::UnknownSound { key } => definition.work_sound.as_ref() == Some(key),
            _ => false,
        })
        .map(|(owner, definition)| format!("mod {owner}, profession {}", definition.key.0))
        .collect::<Vec<_>>();
    format!("{}: {error}", contexts.join(", "))
}
