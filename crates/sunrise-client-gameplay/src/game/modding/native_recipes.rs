//! Recipe lookup, placement, creative gives, and client synchronization. The client uses its
//! own recipes, with server-synced types taking precedence, and follows the server's book for
//! known displays.
use mod_api::CreativeGiveError;
use std::sync::Arc;
use sunrise_gameplay_common::game::api_values::{TryIntoApi, TryIntoEngine};
#[cfg(feature = "client")]
use sunrise_gameplay_common::game::native_recipe_tables::IncomingRecipes;
use sunrise_items::items::ItemStack;

/// A stack's components beyond its item's, as vanilla's item syntax writes
/// them (`[minecraft:potion_contents={potion:"minecraft:swiftness"}]`), or "".
pub(in crate::game::client_side) fn components(s: &ItemStack) -> String {
    use sunrise_core::nbt::Tag;
    let Some(Tag::Compound(entries)) = sunrise_items::item_data::component_snapshot(s) else {
        return String::new();
    };
    let parts: Vec<String> = entries
        .iter()
        .map(|(k, v)| {
            format!(
                "{}={}",
                k.to_string_lossy(),
                sunrise_game_data::snbt::to_snbt(v)
            )
        })
        .collect();
    format!("[{}]", parts.join(","))
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    pub(in crate::game) fn registered_recipe_book_category(
        &mut self,
        kind: &str,
    ) -> Option<mod_api::item::recipe_book::RecipeBookCategory> {
        self.ensure_mod_book_displays();
        self.common
            .recipe_tables
            .books
            .as_ref()?
            .as_ref()
            .ok()?
            .categories()
            .find(|category| category.recipe_type == kind)
            .cloned()
    }
    /// `place_recipe`: refused as `recipes.place` refuses it.
    pub(in crate::game) fn recipe_place(&mut self, book_id: i32, all: bool) -> Result<(), String> {
        if !self.mod_recipe_screen() {
            return Err("no screen that takes recipes is open".into());
        }
        if !self.known_displays().contains(&book_id) {
            return Err(format!(
                "recipe {book_id} is not in the player's recipe book"
            ));
        }
        self.mod_place_recipe(book_id, all);
        Ok(())
    }
    /// `give_creative`: `Inventory.add` of `stack` for a creative player, components kept and up
    /// to the stack's own maximum, then `MultiPlayerGameMode.handleCreativeModeItemAdd` for
    /// every slot it changed, sent now: the end of the tick's sync skips creative slots while
    /// a server menu is open. What did not fit comes back and is not dropped.
    pub fn creative_give_stack(
        &mut self,
        stack: &mod_api::ItemStack,
    ) -> Result<Option<mod_api::ItemStack>, CreativeGiveError> {
        if self.common.is_logical_server()
            || self.common.player.is_none()
            || self.common.res.rest.get().is_none()
        {
            return Err(CreativeGiveError::Unavailable);
        }
        if !self.common.creative {
            return Err(CreativeGiveError::NotCreative);
        }
        let mut stack = stack.try_into_engine(self.items())?;
        if self.client.remote.is_some()
            && !self
                .client
                .creative_screen
                .0
                .catalog()
                .item_enabled(&stack, self.enabled_feature_ids())
        {
            return Err(CreativeGiveError::FeatureDisabled);
        }
        let max = self.stack_max_size(&stack);
        let inventory = self.common.inventory[..36].to_vec();
        let offhand = self.common.offhand.clone();
        self.insert_stack_up_to(&mut stack, max);
        for i in 0..36 {
            if self.common.inventory[i] != inventory[i] {
                self.remote_creative_slot_add(i);
            }
        }
        if self.common.offhand != offhand {
            self.remote_creative_slot_add(36);
        }
        if stack.count == 0 {
            return Ok(None);
        }
        stack
            .try_into_api()
            .map(Some)
            .map_err(|error| error.into_write_error().into())
    }
    /// The `book_id`s the player's recipe book holds now.
    pub(in crate::game) fn known_displays(&mut self) -> Arc<[i32]> {
        if self.common.res.rest.get().is_none() {
            return Arc::from([]);
        }
        let (version, known) = self.recipe_book_displays();
        if self.common.recipe_tables.known.0 != Some(version) {
            self.common.recipe_tables.known = (Some(version), known.into());
        }
        self.common.recipe_tables.known.1.clone()
    }

    /// `RecipesClientHost::recipe_book_ids`: the display ids the recipe book
    /// gives `recipe_id`, as `transfer_recipe` looks them up.
    pub(in crate::game) fn recipe_book_ids(&mut self, recipe_id: &str) -> Arc<[i32]> {
        if self.common.player.is_none() || self.common.res.rest.get().is_none() {
            return Arc::from([]);
        }
        self.viewer_book_ids(recipe_id)
    }
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// A new play session (`Login`): what the last server sent is gone.
    pub(in crate::game::client_side) fn forget_synced_recipes(&mut self) {
        self.common.recipe_tables.synced = None;
        self.common.recipe_tables.synced_index = None;
        self.common.recipe_tables.incoming = IncomingRecipes::default();
        self.common.recipe_tables.received = None;
        self.forget_book();
    }
    /// `native_recipes_received` for the types that came, once the window
    /// has its player: the recipes of a join come before it, and the helpers
    /// answer nothing until then. Once a frame.
    pub(in crate::game::client_side) fn recipes_received_now(&mut self) {
        if self.common.player.is_some()
            && let Some(types) = self.common.recipe_tables.received.take()
        {
            self.native_recipes_received(&types);
            self.build_screen();
        }
    }
    /// A `sunrise:recipe_content` part the server sent the window. On the
    /// last, the window's tables take the server's recipes of those types in
    /// place of its own, and the mods hear of it (`RecipesReceivedEvent`).
    pub(in crate::game::client_side) fn remote_recipe_content(&mut self, data: &[u8]) {
        let part = match sunrise_sim::lan::recipes::ContentPart::decode(data) {
            Ok(part) => part,
            Err(e) => {
                log::warn!("Sunrise: bad recipe content from the server: {e}");
                return;
            }
        };
        let (received, types) = match self.common.recipe_tables.incoming.push(part) {
            None => return,
            Some(Ok(update)) => update,
            Some(Err(why)) => {
                log::warn!("Sunrise: the server's recipes were not taken: {why}");
                return;
            }
        };
        // No type any more: back on the window's own recipes.
        if types.is_empty() {
            self.common.recipe_tables.synced_index = None;
            if self.common.recipe_tables.synced.take().is_some() {
                self.forget_book();
                self.common.recipe_tables.received = Some(types);
                self.recipes_received_now();
            }
            return;
        }
        let built = self
            .common
            .res
            .recipes()
            .with_recipe_content(self.items(), received, &types);
        match built {
            Ok((recipes, index)) => {
                self.common.recipe_tables.synced = Some(recipes);
                self.common.recipe_tables.synced_index = Some(index);
                self.forget_book();
                self.common.recipe_tables.received = Some(types);
                self.recipes_received_now();
            }
            Err(e) => log::warn!("Sunrise: the server's recipes were not taken: {e}"),
        }
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt1 as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::native_book_displays::NativeBookDisplaysGameExt1 as _;

#[cfg(feature = "client")]
#[cfg(test)]
mod tests {
    use sunrise_gameplay_common::game::tables::GameItems as _;
    /// A part with a malformed serializer refuses the whole update: the
    /// valid last part after it publishes nothing, one refusal is reported
    /// when the update completes, and the next update is taken whole.
    #[test]
    fn a_bad_part_refuses_the_whole_update_at_its_last_part() {
        use sunrise_gameplay_common::game::native_recipe_tables::IncomingRecipes;
        use sunrise_sim::lan::recipes::ContentPart;
        let part = |text: &str| ContentPart::decode(text.as_bytes()).unwrap();
        let mut incoming = IncomingRecipes::default();
        assert!(
            incoming
                .push(part(
                    r#"{"recipes":[["a:one",{"type":"smelting"}]],"last":null}"#
                ))
                .is_none()
        );
        assert!(
            incoming
                .push(part(
                    r#"{"recipes":[["a:bad",{"type":"x:y:z"}]],"last":null}"#
                ))
                .is_none()
        );
        let refused = incoming.push(part(
            r#"{"recipes":[["a:two",{"type":"smelting"}]],"last":["minecraft:smelting"]}"#,
        ));
        assert!(refused.unwrap().unwrap_err().contains("a:bad"));
        let (recipes, types) = incoming
            .push(part(
                r#"{"recipes":[["a:three",{"type":"smelting"}]],"last":["minecraft:smelting"]}"#,
            ))
            .unwrap()
            .unwrap();
        assert_eq!(recipes.len(), 1);
        assert_eq!(recipes[0].0, "a:three");
        assert_eq!(types, ["minecraft:smelting"]);
    }

    use super::super::native_entity_refs::tests::{with_client_game, with_game_role_client};

    /// With the server's crafting recipes synced, the book still holds the
    /// vanilla ones under their own display ids (a viewer's "+" reads them,
    /// `recipe_book_ids`), and a recipe only the server has gets one too.
    #[test]
    #[ignore = "daily"]
    fn synced_vanilla_recipes_keep_their_book_ids() {
        fn book_ids(game: &mut crate::game::ClientGame<'_>, id: &str) -> Vec<i32> {
            (game.recipe_book_entries().into_iter())
                .filter(|e| e.id == id && e.display_id >= 0)
                .map(|e| e.display_id)
                .collect()
        }
        with_client_game(|game| {
            let ids = [
                "minecraft:stick",
                "minecraft:chest",
                "minecraft:iron_ingot_from_smelting_raw_iron",
            ];
            let own = ids.map(|id| book_ids(game, id));
            assert!(own.iter().all(|ids| !ids.is_empty()), "{own:?}");
            let types = ["minecraft:crafting", "minecraft:smelting"].map(str::to_owned);
            let pack = serde_json::json!({
                "type": "minecraft:crafting_shaped",
                "pattern": ["#", "#"],
                "key": {"#": "#minecraft:logs"},
                "result": {"id": "minecraft:stick", "count": 16},
            });
            let overlay = sunrise_world::loot::PackOverlay::NONE;
            let mut files =
                sunrise_items::recipe_sources::sources(&game.options.resources, &overlay).unwrap();
            files.push(("zzz:sticks_from_logs".to_owned(), pack));
            let server = sunrise_sim::crafting::Recipes::from_sources(
                &game.options.resources,
                game.items(),
                &overlay,
                files,
                game.res.core.item_data.context(),
            )
            .unwrap();
            let sources = server.recipe_content(&types);
            for part in sunrise_sim::lan::recipes::encode_content(&types, &sources, false) {
                game.remote_recipe_content(&part);
            }
            assert!(game.synced_recipes().is_some());
            assert_eq!(ids.map(|id| book_ids(game, id)), own);
            assert_eq!(book_ids(game, "zzz:sticks_from_logs").len(), 1);
        });
    }

    /// A content mod that ships files under `minecraft` (tags, as most do)
    /// doesn't make vanilla's namespace one of the root's content namespaces:
    /// listed, `recipe_sources::sources` read every vanilla recipe twice, as
    /// `stick` and `minecraft:stick`, and the book held two entries of each,
    /// numbered `n` and `n + 1`, so a viewer's "+" sent the neighbour's id. A
    /// vanilla recipe has exactly one book id, the server's, synced or not.
    #[test]
    #[ignore = "daily"]
    fn a_synced_vanilla_recipe_has_one_book_id_the_servers() {
        use std::path::{Path, PathBuf};
        fn book_ids(game: &mut crate::game::ClientGame<'_>, id: &str) -> Vec<i32> {
            (game.recipe_book_entries().into_iter())
                .filter(|e| e.id == id && e.display_id >= 0)
                .map(|e| e.display_id)
                .collect()
        }
        let base = PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let scratch =
            std::env::temp_dir().join(format!("sunrise-one-book-id-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        let dir = scratch.join("tagger");
        for (name, text) in [
            (
                "sunrise-mod.toml",
                "id = \"tagger\"\nname = \"tagger\"\nversion = \"1.0\"\napi = \"0.2\"\ncapabilities = [\"registries\"]\n",
            ),
            (
                "data/minecraft/tags/item/tagger_extra.json",
                r#"{"values": ["minecraft:stick"]}"#,
            ),
        ] {
            let path = dir.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let package = sunrise_mod_package::Package::read(&dir).unwrap();
        let tagger = sunrise_sim::mod_content::ContentMod {
            id: package.manifest.id.clone(),
            content: sunrise_mod_package::content::Content::read(&dir, &package.manifest).unwrap(),
            jars: Vec::new(),
        };
        let root = scratch.join("root");
        let inventory = sunrise_mod_package::enabled::EnabledMods::new([(
            "tagger".to_owned(),
            mod_api::Side::Both,
        )]);
        sunrise_sim::mod_content::build(&base, Path::new(""), &[tagger], &root, &inventory)
            .unwrap();
        assert_eq!(sunrise_sim::mod_content::namespaces(&root), ["tagger"]);
        with_game_role_client(
            root,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::Client,
            |game| {
                let ids = [
                    "minecraft:stick",
                    "minecraft:stick_from_bamboo_item",
                    "minecraft:chest",
                    "minecraft:iron_ingot_from_smelting_raw_iron",
                ];
                // The integrated server numbers the book's displays by its index.
                let server = ids.map(|id| {
                    let index = &game.stations().recipe_index;
                    vec![index.first_display(id).unwrap()]
                });
                assert_eq!(ids.map(|id| book_ids(game, id)), server);
                let types = ["minecraft:crafting", "minecraft:smelting"].map(str::to_owned);
                let sources = game.recipes().recipe_content(&types);
                for part in sunrise_sim::lan::recipes::encode_content(&types, &sources, false) {
                    game.remote_recipe_content(&part);
                }
                assert!(game.synced_recipes().is_some());
                assert_eq!(ids.map(|id| book_ids(game, id)), server);
            },
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }
}
