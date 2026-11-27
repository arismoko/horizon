#![cfg(test)]
#[cfg(test)]
pub(crate) mod lectern_book_removal_tests {
    use super::super::game_tests::tests::server_game;
    use sunrise_items::items::ItemStack;
    use sunrise_level::containers::{BlockEntity, Lectern};

    /// `LecternBlock.resetBookState`: removing a book clears `has_book` and posts `block_change`.
    #[test]
    #[ignore = "daily"]
    fn a_lectern_book_removal_posts_block_change() {
        let mut game_resources = None;
        let mut game = server_game(&mut game_resources);
        let key = (2, 4, 3);
        let state = game
            .pack
            .assets
            .registry
            .default_state("minecraft:lectern")
            .unwrap();
        game.world.set(key.0, key.1, key.2, state).unwrap();
        game.world.block_entities.insert(
            key,
            BlockEntity::Lectern(Lectern {
                book: Some(ItemStack::new("minecraft:written_book", 1)),
                page: 0,
            }),
        );
        game.set_lectern_property(key, "has_book", true);
        sunrise_gameplay_common::game::side_ops::events::TEST_POSTED_EVENTS
            .with(|e| *e.borrow_mut() = Some(Vec::new()));

        game.set_lectern_has_book(key, false, None);

        let has_book = game
            .pack
            .assets
            .registry
            .state(game.world.block(key.0, key.1, key.2))
            .and_then(|s| s.properties.get("has_book").cloned());
        assert_eq!(
            has_book.as_deref(),
            Some("false"),
            "the lectern has no book"
        );
        sunrise_gameplay_common::game::side_ops::events::TEST_POSTED_EVENTS.with(|e| {
            assert!(
                e.borrow().as_ref().unwrap().contains(&(
                    "minecraft:block_change",
                    [2.5, 4.5, 3.5],
                    None
                )),
                "{:?}",
                e.borrow()
            );
        });
    }
}

#[cfg(test)]
pub(crate) mod lectern_books_tag_tests {
    use super::super::game_tests::tests::server_game;
    use sunrise_items::items::ItemStack;
    use sunrise_level::containers::BlockEntity;

    /// `LecternBlock.useItemOn` checks `ItemTags.LECTERN_BOOKS`; a world's
    /// data pack adds its own book to that tag, and the lectern takes it.
    #[test]
    #[ignore = "daily"]
    fn a_packs_lectern_books_entry_goes_onto_the_lectern() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        let dir =
            std::env::temp_dir().join(format!("sunrise-lectern-books-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let tags = dir.join("datapacks/books/data/minecraft/tags/item");
        std::fs::create_dir_all(&tags).unwrap();
        std::fs::write(
            dir.join("datapacks/books/pack.mcmeta"),
            r#"{"pack":{"description":"fixture"}}"#,
        )
        .unwrap();
        std::fs::write(
            tags.join("lectern_books.json"),
            r#"{"values":["minecraft:written_book","minecraft:writable_book","minecraft:book"]}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("datapacks.json"),
            r#"{"enabled":["file/books"],"disabled":[]}"#,
        )
        .unwrap();
        game.server.world_root = dir;
        let stack = game.reload_datapacks(false, &[]).unwrap();
        let root = game.options.resources.clone();
        game.server.world_tables.0 = game
            .stage_world_tables(&stack, false, &root)
            .unwrap()
            .tables;

        let state = game
            .pack
            .assets
            .registry
            .default_state("minecraft:lectern")
            .unwrap();
        let key = (8, 64, 8);
        game.world.set(key.0, key.1, key.2, state).unwrap();

        // A book the pack put in the tag is placed; one it did not is not.
        let stone = ItemStack::new("minecraft:stone", 1);
        assert!(!game.use_lectern([8, 64, 8], Some(&stone)));
        let book = ItemStack::new("minecraft:book", 1);
        assert!(
            game.use_lectern([8, 64, 8], Some(&book)),
            "the pack's `#minecraft:lectern_books` entry was refused"
        );
        let Some(BlockEntity::Lectern(lectern)) = game.world.block_entities.get(&key) else {
            panic!("no lectern block entity");
        };
        assert_eq!(
            lectern.book.as_ref().map(|b| &*b.item),
            Some("minecraft:book")
        );
    }
}
