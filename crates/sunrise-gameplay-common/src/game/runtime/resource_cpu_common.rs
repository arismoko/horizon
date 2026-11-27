use crate::game::resource_load::named_scope;
use std::sync::Arc;
use sunrise_assets::resource_source::ResourceRoot;
use sunrise_game_data::Result;
use sunrise_gameplay_common::painting_variants::PaintingVariants;
use sunrise_items::creative_catalog::{
    Catalog, ItemFeatureRequirements, Kind, parse_search_metadata_stack,
};

pub struct ServerCpuResources {
    pub painting_variants: PaintingVariants,
    pub item_features: ItemFeatureRequirements,
    pub creative_catalog: Arc<Catalog>,
}

impl ServerCpuResources {
    pub fn load(resources: &ResourceRoot) -> Result<Self> {
        let root = resources.prepared();
        let (painting_variants, creative_catalog) = std::thread::scope(|scope| {
            let painting_variants = named_scope(scope, "game_cpu.paintings", move || {
                PaintingVariants::load(root)
            });
            let creative_catalog = named_scope(scope, "game_cpu.creative", move || {
                let raw: serde_json::Value = serde_json::from_slice(
                    &std::fs::read(root.join("creative-inventory.json"))
                        .map_err(|error| error.to_string())?,
                )
                .map_err(|error| error.to_string())?;
                let catalog = Catalog::parse(&raw)?;
                for (tab, recipe) in raw["tabs"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .zip(catalog.tabs())
                {
                    if recipe.kind == Kind::Search {
                        for row in tab["search_metadata"].as_array().into_iter().flatten() {
                            parse_search_metadata_stack(row)?;
                        }
                    }
                }
                Ok::<_, String>(Arc::new(catalog))
            });
            (
                painting_variants
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
                creative_catalog
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
            )
        });
        let creative_catalog = creative_catalog?;
        let item_features = creative_catalog.item_features();
        Ok(Self {
            painting_variants,
            item_features,
            creative_catalog,
        })
    }
}
