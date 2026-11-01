fn sample_receive(game: &Game<'_>, recipe_types: &[String]) {
    let recipes = game.native_recipes_of_type("machine_sample:processing");
    let ids: Vec<_> = recipes.iter().map(|r| r.id.as_str()).collect();
    let control = game.native_recipes_of_type("machine_sample:control").len();
    eprintln!("RECEIVE_TEST types={recipe_types:?} count={} ids={ids:?} control={control}", recipes.len());
    let book = horizon_client::recipe_book::Book::build(game.recipes(), game.items());
    let book_ids: Vec<_> = book.entries.iter().filter(|e| e.id == "minecraft:oak_planks" || e.id.starts_with("machine_sample:")).map(|e| &e.id).collect();
    eprintln!("BOOK_TEST ids={book_ids:?}");
}
