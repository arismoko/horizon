//! Tag-only item table construction.
use super::*;
impl Items {
    /// Only the block and item tags (and the enchantment registry, which the
    /// station rules and loot functions are built from), for tools that need
    /// tag membership.
    pub fn tags_only(root: &Path, overlay: &PackOverlay, defaults: &ItemDefaults) -> Result<Self> {
        let block_items = block_items(root)?;
        let (component_registry, entity_type_tags) =
            component_tables(root, overlay, LoadEntityTags::Prepared)?;
        let mut items = Self {
            component_registry,
            entity_type_tags,
            component_defaults: component_defaults(defaults)?,
            ..Self::default()
        };
        items.bind_tags(root, overlay, defaults, |id| block_items.contains_key(id))?;
        items.install_registries(
            crate::enchant::Enchantments::load_definitions(root, overlay, defaults)?,
            HashMap::new(),
        );
        items.bind_enchantment_tags(root, overlay)?;
        load_component_damage_registry(root, overlay, &mut items)?;
        Ok(items)
    }

    /// Bind ingredient codecs to the world's frozen damage registry and reloaded tags.
    /// Do this before preparing recipes, alongside binding item damage predicates.
    pub fn bind_component_damage_catalog(
        &mut self,
        catalog: &sunrise_world::damage_source::Catalog,
    ) {
        let mut ids: Vec<_> = catalog
            .entries()
            .map(|(_, entry)| entry.name.clone())
            .collect();
        ids.sort();
        let registry = Arc::make_mut(&mut self.component_registry);
        registry.by_registry.insert(
            "minecraft:damage_type".into(),
            sunrise_game_data::registries::RegistryIds {
                ids,
                tags: catalog.tags().map(str::to_owned).collect(),
            },
        );
        let ordered = catalog
            .tags()
            .map(|tag| {
                (
                    tag.to_owned(),
                    catalog.tag_members(tag).map(str::to_owned).collect(),
                )
            })
            .collect();
        self.set_tags(TagRegistry::DamageType, ordered);
    }

    /// Bind a damage type registry received rather than loaded, a client's from its server:
    /// its type names and its tags' members, where [`Self::bind_component_damage_catalog`] and
    /// [`Enchantments::bind_damage_catalog`](crate::enchant::Enchantments::bind_damage_catalog)
    /// put a loaded catalog's.
    pub fn bind_damage_registry(
        &mut self,
        mut ids: Vec<String>,
        tags: HashMap<String, Vec<String>>,
    ) {
        ids.sort();
        let mut names: Vec<_> = tags.keys().cloned().collect();
        names.sort();
        Arc::make_mut(&mut self.component_registry)
            .by_registry
            .insert(
                "minecraft:damage_type".into(),
                sunrise_game_data::registries::RegistryIds { ids, tags: names },
            );
        Arc::make_mut(&mut self.enchantments).damage_tags = tags
            .iter()
            .map(|(tag, members)| (tag.clone(), members.iter().cloned().collect()))
            .collect();
        self.set_tags(TagRegistry::DamageType, tags);
    }
}

/// Standalone catalogs have no world-owned snapshot; load their complete pack registry.
pub(super) fn load_component_damage_registry(
    root: &Path,
    overlay: &PackOverlay,
    items: &mut Items,
) -> Result<()> {
    if root.join("damage-catalog.json").exists() {
        let catalog = sunrise_world::damage_source::Catalog::load(root, overlay)?;
        items.bind_component_damage_catalog(&catalog);
    }
    Ok(())
}

/// Preserve the exact loaded prototype, including native default-component changes.
pub(super) fn component_defaults(
    defaults: &ItemDefaults,
) -> Result<Arc<HashMap<String, sunrise_core::nbt::Compound>>> {
    Ok(sunrise_world::loot::copy::component_defaults(defaults)?)
}

/// Catalogs also need Block.getItem for block-tag ingredients, without full block facts.
pub(super) fn block_items(root: &Path) -> Result<HashMap<String, String>> {
    #[derive(serde::Deserialize)]
    struct Registry {
        defaults: HashMap<String, usize>,
    }
    #[derive(serde::Deserialize)]
    struct Gameplay {
        states: Vec<State>,
    }
    #[derive(serde::Deserialize)]
    struct State {
        item: Option<String>,
    }
    let open = |name: &str| match fs::File::open(root.join(name)) {
        Ok(file) => Ok(Some(file)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    };
    let (Some(registry), Some(gameplay)) = (open("facts.json")?, open("gameplay-facts.json")?)
    else {
        return Ok(HashMap::new());
    };
    let registry: Registry = serde_json::from_reader(std::io::BufReader::new(registry))?;
    let gameplay: Gameplay = serde_json::from_reader(std::io::BufReader::new(gameplay))?;
    registry
        .defaults
        .into_iter()
        .map(|(block, state)| {
            let item = gameplay
                .states
                .get(state)
                .ok_or("block default state is missing gameplay facts")?
                .item
                .as_deref()
                .unwrap_or("minecraft:air");
            Ok((block, item.to_owned()))
        })
        .collect()
}

/// Which entity registry to use when binding this item generation's tags.
pub enum LoadEntityTags<'a> {
    Prepared,
    Frozen(&'a Arc<[String]>),
    Reload(&'a Arc<EntityTypeTags>),
}

/// The entity membership and registry metadata used by ingredient component codecs.
pub(crate) fn component_tables(
    root: &Path,
    overlay: &PackOverlay,
    source: LoadEntityTags<'_>,
) -> Result<(
    Arc<sunrise_game_data::registries::Registries>,
    Arc<EntityTypeTags>,
)> {
    let mut registry = sunrise_game_data::registries::Registries::default();
    let path = root.join("command-registries.json");
    if path.exists() {
        registry.fill_from_export(&serde_json::from_slice(&fs::read(path)?)?);
    }
    let path = root.join("registries.json");
    if path.exists() {
        let json: Value = serde_json::from_slice(&fs::read(path)?)?;
        if let Some(entries) = json["minecraft:data_component_type"]["entries"].as_object() {
            registry
                .by_registry
                .entry("minecraft:data_component_type".into())
                .or_default()
                .ids = entries.keys().cloned().collect();
        }
    }
    let entity_type_tags = match source {
        LoadEntityTags::Prepared => {
            // The prepared entity table includes content-mod types, unlike the command export.
            let ids = match fs::read(root.join("entity-types.json")) {
                Ok(bytes) => EntityTypeTags::registry_ids(&serde_json::from_slice(&bytes)?)?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    Arc::from(registry.ids(EntityTypeTags::REGISTRY))
                }
                Err(error) => return Err(error.into()),
            };
            Arc::new(EntityTypeTags::load(root, overlay, ids)?)
        }
        LoadEntityTags::Frozen(ids) => Arc::new(EntityTypeTags::load(root, overlay, ids.clone())?),
        LoadEntityTags::Reload(previous) => {
            let tags = EntityTypeTags::load(root, overlay, previous.ids().clone())?;
            // TagLoader.loadPendingTags keeps the prior bindings if none built.
            // A successfully built empty tag is still a replacement.
            if tags.ordered().is_empty() {
                previous.clone()
            } else {
                Arc::new(tags)
            }
        }
    };
    let entities = registry
        .by_registry
        .entry(EntityTypeTags::REGISTRY.into())
        .or_default();
    entities.ids = entity_type_tags.ids().to_vec();
    // These are the current merged pack tags, not the export's stale tag names.
    entities.tags = entity_type_tags.ordered().keys().cloned().collect();
    entities.tags.sort();
    // The command export is vanilla-only. World recipe loads bind their active
    // frozen catalog; standalone catalogs load the full pack/mod catalog above.
    registry.damage_types.clear();
    registry.by_registry.insert(
        "minecraft:damage_type".into(),
        sunrise_game_data::registries::RegistryIds::default(),
    );
    Ok((Arc::new(registry), entity_type_tags))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_reload_retains_unbound_tags_but_applies_bound_empty_tags() {
        let root = std::env::temp_dir().join(format!("sunrise-entity-tags-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let tag = "test:walkers";
        let cow = "minecraft:cow";
        let previous = Arc::new(EntityTypeTags::from_ordered(
            [(tag.into(), vec![cow.into()])].into(),
            [cow.into()].into(),
        ));
        let reload = |bytes: &[u8], reject_builtins: bool| {
            let mut files = vec![("tags/test:entity_type/walkers".into(), vec![bytes.to_vec()])];
            if reject_builtins {
                // Built-in empty convention tags also bind unless rejected by the pack stack.
                for id in [
                    "tags/c:entity_type/capturing_not_supported",
                    "tags/c:entity_type/teleporting_not_supported",
                ] {
                    files.push((
                        id.into(),
                        vec![br#"{"replace":true,"values":["test:missing"]}"#.to_vec()],
                    ));
                }
            }
            component_tables(
                &root,
                &PackOverlay::new(&files),
                LoadEntityTags::Reload(&previous),
            )
            .unwrap()
            .1
        };
        assert!(reload(br#"{"values":["test:missing"]}"#, true).contains(tag, cow));
        assert_eq!(
            reload(br#"{"values":[]}"#, false).ordered().get(tag),
            Some(&Vec::new())
        );
        std::fs::remove_dir(root).unwrap();
    }
}
