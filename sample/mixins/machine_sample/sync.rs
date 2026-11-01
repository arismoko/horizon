fn sample_sync(recipe_types: &mut Vec<String>) {
    let ask = std::env::var_os("RECIPE_SYNC_ASK").is_some();
    if ask { recipe_types.push("machine_sample:processing".to_owned()); }
    eprintln!("SYNC_TEST ask={ask}");
}
