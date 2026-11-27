#[cfg(test)]
#[path = "tests/lectern_tests.rs"]
pub(in crate::game) mod lectern_tests;

#[cfg(test)]
pub(in crate::game) mod lectern_signal_tests {
    use crate::game::server_side::game_tests::tests::server_game;
    use sunrise_core::position::BlockPos;
    use sunrise_gameplay_common::game::GameGameExt1 as _;
    use sunrise_gameplay_common::lectern::lectern_signal;
    use sunrise_items::{item_data::WrittenBook, items::ItemStack};
    use sunrise_level::containers::{BlockEntity, Lectern};
    use sunrise_world::level::LevelAccess;
    use sunrise_world::registry::BlockStateId;

    fn signal(
        game: &mut crate::game::ServerGame<'_>,
        has_book: bool,
        book: Option<ItemStack>,
        page: i32,
    ) -> i32 {
        let key = (8, 64, 8);
        let registry = game.pack.assets.registry;
        let default = registry.default_state("minecraft:lectern").unwrap();
        let mut state = registry.state(default).unwrap().clone();
        state
            .properties
            .insert("has_book".into(), has_book.to_string());
        let state: BlockStateId = registry.id(&state).unwrap();
        game.world.set(key.0, key.1, key.2, state).unwrap();
        game.world
            .block_entities
            .insert(key, BlockEntity::Lectern(Lectern { book, page }));
        let pos = BlockPos::new(key.0, key.1, key.2);
        lectern_signal(&game.world, registry, state, pos).unwrap()
    }

    /// `LecternBlock.getAnalogOutputSignal` reads the block entity only while
    /// the state's `HAS_BOOK` is true.
    #[test]
    #[ignore = "daily"]
    fn a_lectern_without_the_has_book_state_feeds_no_comparator() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        let key = (8, 64, 8);
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
                book: Some(ItemStack::new("minecraft:writable_book", 1)),
                page: 0,
            }),
        );
        let signal = |game: &mut crate::game::ServerGame<'_>, state| {
            game.with_level(|_, level, _| {
                level.block_entity_analog_signal(
                    BlockPos::new(key.0, key.1, key.2),
                    state,
                    sunrise_core::geometry::Direction::North,
                )
            })
        };
        assert_eq!(
            signal(&mut game, state),
            0,
            "the block state says the lectern holds no book"
        );
        let registry = game.pack.assets.registry;
        let mut with_book = registry.state(state).unwrap().clone();
        with_book
            .properties
            .insert("has_book".into(), "true".into());
        let with_book = registry.id(&with_book).unwrap();
        game.world.set(key.0, key.1, key.2, with_book).unwrap();
        assert_eq!(
            signal(&mut game, with_book),
            15,
            "with HAS_BOOK the book's page signal"
        );
    }

    /// `LecternBlock.getAnalogOutputSignal` asks for the state's `HAS_BOOK`
    /// first, so a lectern without it reads 0 whatever its block entity holds;
    /// with it, `LecternBlockEntity.getRedstoneSignal` is `floor(page /
    /// (pageCount - 1) * 14) + (hasBook() ? 1 : 0)`. `hasBook()` asks for a
    /// book-content component, so a stack that only sits in the lectern (a
    /// non-book item a command left in `Book`) reads 14, not the book's 15.
    /// An entity with no book at all reads 14 too: a page count of 0 is a page
    /// progress of 1.0 with no book to add.
    #[test]
    #[ignore = "daily"]
    fn a_lecterns_signal_adds_one_only_for_a_book() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        let writable = Some(ItemStack::new("minecraft:writable_book", 1));
        let mut written = ItemStack::new("minecraft:written_book", 1);
        written.data_mut().written = Some(WrittenBook {
            title: "T".into(),
            author: "A".into(),
            pages: vec!["one".into(), "two".into()],
            ..Default::default()
        });
        // `has_book=false`: no signal, whatever the block entity holds.
        assert_eq!(signal(&mut game, false, None, 0), 0);
        assert_eq!(
            signal(&mut game, false, writable.clone(), 0),
            0,
            "state denies"
        );
        // `has_book=true`, an entity with no book: page progress 1.0.
        assert_eq!(signal(&mut game, true, None, 0), 14, "no book");
        let stone = Some(ItemStack::new("minecraft:stone", 1));
        assert_eq!(signal(&mut game, true, stone, 0), 14, "no book content");
        // `writable_book_content` is a writable book's default component.
        assert_eq!(signal(&mut game, true, writable, 0), 15, "a book");
        assert_eq!(
            signal(&mut game, true, Some(written.clone()), 0),
            1,
            "page 0"
        );
        assert_eq!(signal(&mut game, true, Some(written), 1), 15, "last page");
    }
}
