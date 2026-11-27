//! A custom crafting remainder reaches the normal server toss moment.
use std::result::Result;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use sunrise_gameplay_common::game::ItemStack;
pub(crate) struct Probe {
    pub tosses: Arc<AtomicUsize>,
    pub(crate) bad_recipe: Arc<AtomicBool>,
}
pub(crate) struct Kind {
    pub stick: mod_api::ItemKey,
    pub(crate) diamond: mod_api::ItemKey,
    pub(crate) bucket: mod_api::ItemKey,
}
impl mod_api::CraftingRecipeKind for Kind {
    type Value = u32;
    fn decode(&self, _: &str) -> Result<u32, String> {
        Ok(3)
    }
    fn write(&self, _: &u32) -> Result<String, String> {
        Ok("{}".into())
    }
    fn matches(
        &self,
        _context: &dyn mod_api::LoopContext,
        _: &u32,
        grid: mod_api::CraftingGrid,
    ) -> bool {
        grid.cells.iter().enumerate().all(|(i, s)| {
            if i == 0 {
                s.as_ref().is_some_and(|s| s.item() == self.stick)
            } else {
                s.is_none()
            }
        })
    }
    fn assemble(
        &self,
        _context: &dyn mod_api::LoopContext,
        value: &u32,
        _: mod_api::CraftingGrid,
    ) -> Option<mod_api::ItemStack> {
        mod_api::ItemStack::new(self.diamond, *value)
    }
    fn remaining_items(
        &self,
        _context: &dyn mod_api::CraftingRemainders,
        _: &u32,
        grid: mod_api::CraftingGrid,
    ) -> Option<Vec<Option<mod_api::ItemStack>>> {
        let mut out = vec![None; grid.cells.len()];
        out[0] = mod_api::ItemStack::new(self.bucket, 1);
        Some(out)
    }
}
#[mod_api::__subscriptions]
impl mod_api::Hooks for Probe {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let kind = Kind {
            stick: r.item_key("minecraft:stick"),
            diamond: r.item_key("minecraft:diamond"),
            bucket: r.item_key("minecraft:bucket"),
        };
        r.register_crafting_recipe_serializer("literal", kind);
        r.register_recipe_type("machine_type");
        r.register_raw_recipe_serializer("machine_json", "recipe_probe:machine_type");
    }
    fn data_pack_files(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::DataPackFilesEvent<'_>,
    ) {
        if self.bad_recipe.load(Ordering::Relaxed) {
            event.files.push((
                "recipe/recipe_probe:bad".into(),
                br#"{"type":"missing:serializer"}"#.to_vec(),
            ));
        }
    }
    fn item_toss(
        &self,
        _: &mut dyn mod_api::ServerHost,
        _: &mut mod_api::ItemTossEvent,
    ) -> mod_api::Flow {
        self.tosses.fetch_add(1, Ordering::Relaxed);
        mod_api::Flow::Continue
    }
}
#[test]
#[ignore = "daily"]
fn registered_crafting_remainder_drops_through_server_item_toss() {
    let tosses = Arc::new(AtomicUsize::new(0));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "recipe_probe",
        Box::new(Probe {
            tosses: tosses.clone(),
            bad_recipe: Arc::new(AtomicBool::new(false)),
        }),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        sunrise_gameplay::game::native_guest_turn_tests::with_guests(|game| {
            let recipes = sunrise_sim::crafting::Recipes::from_sources(
                &game.options.resources,
                game.items(),
                &sunrise_world::loot::PackOverlay::NONE,
                vec![
                    (
                        "recipe_probe:literal".into(),
                        serde_json::json!({"type":"recipe_probe:literal"}),
                    ),
                    (
                        "recipe_probe:machine_recipe".into(),
                        serde_json::json!({"type":"recipe_probe:machine_json","duration":42}),
                    ),
                ],
                game.res.core.item_data.context(),
            )
            .unwrap();
            let scratch = tempfile::tempdir().unwrap();
            game.server.world_root = scratch.path().to_path_buf();
            game.reload_datapacks(false, &[]).unwrap();
            game.test_set_world_recipes(recipes);
            let host: &dyn mod_api::Host = game;
            let machine = host.recipes_of_type("recipe_probe:machine_type");
            assert_eq!(machine.len(), 1);
            assert_eq!(machine[0].kind, "recipe_probe:machine_type");
            assert_eq!(machine[0].json["type"], "recipe_probe:machine_json");
            assert_eq!(machine[0].json["duration"], 42);
            assert!(host.recipes_of_type("recipe_probe:machine_json").is_empty());
            assert_eq!(game.recipes().crafting.len(), 1);
            let level = game.level(game.dim.id()).unwrap();
            let kind = mod_api::Identifier::parse("minecraft:crafting").unwrap();
            let stick = mod_api::ItemStack::new(
                mod_api::__private::item_key("minecraft:stick").unwrap(),
                2,
            );
            let input = mod_api::RecipeInput::Crafting {
                width: 1,
                height: 2,
                slots: vec![stick, None],
            };
            assert!(matches!(
                level.recipe_for(
                    game,
                    &mod_api::Identifier::parse("recipe_probe:machine_type").unwrap(),
                    &input
                ),
                Err(mod_api::RecipeLookupError::NotMatchable)
            ));
            let matched = level.recipe_for(game, &kind, &input).unwrap().unwrap();
            assert_eq!(matched.output.count(), 3);
            assert_eq!(
                Some(matched.output.item()),
                mod_api::__private::item_key("minecraft:diamond")
            );
            assert_eq!(matched.remainders.len(), 2);
            assert_eq!(matched.remainders[0].as_ref().map(|s| s.count()), Some(1));
            assert!(matched.remainders[1].is_none());
            assert!(matches!(
                level.recipe_for(
                    game,
                    &mod_api::Identifier::parse("recipe_probe:unknown").unwrap(),
                    &input
                ),
                Err(mod_api::RecipeLookupError::NotMatchable)
            ));
            let ids = vec!["recipe_probe:literal".to_owned()];
            assert_eq!(game.change_recipes(true, &ids), 1);
            assert_eq!(game.change_recipes(true, &ids), 0);
            assert!(
                game.level
                    .recipe_book
                    .recipes
                    .contains("recipe_probe:literal")
            );
            assert_eq!(game.change_recipes(false, &ids), 1);
            assert!(
                !game
                    .level
                    .recipe_book
                    .recipes
                    .contains("recipe_probe:literal")
            );
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |guest| {
                    guest.game_mode = 0;
                    guest.carried = None;
                    guest.inventory = vec![Some(ItemStack::new("minecraft:stone", 64)); 36];
                    guest.craft_grid = Default::default();
                    guest.craft_grid[0] = Some(ItemStack::new("minecraft:stick", 2));
                });
            game.lan_guest_packet(
                12,
                sunrise_protocol::packets::play::Serverbound::ContainerClick(
                    sunrise_protocol::packets::play::ContainerClick {
                        container: 0,
                        state: 0,
                        slot: 0,
                        button: 0,
                        input: 0,
                        changed: Vec::new(),
                        carried: None,
                    },
                ),
            );
            let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
            assert_eq!(
                guest.carried.as_ref().map(|s| (&*s.item, s.count)),
                Some(("minecraft:diamond", 3))
            );
            assert_eq!(guest.craft_grid[0].as_ref().map(|s| s.count), Some(1));
            assert_eq!(tosses.load(Ordering::Relaxed), 1);
        });
    });
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
