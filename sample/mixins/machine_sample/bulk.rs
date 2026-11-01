fn sample_bulk(game: &mut Game<'_>, dimension: &str) {
    if dimension != "minecraft:overworld" { return; }
    let path = "/box/work/sample-5905/action";
    let Ok(action) = std::fs::read_to_string(path) else { return; };
    let _ = std::fs::remove_file(path);
    let p: Vec<_> = action.split_whitespace().collect();
    if p.is_empty() { return; }
    use super::native_world_edit::{NativeBlockEdit, NativeBlockRun};
    if p[0] == "singlepipe" {
        let mut reader = horizon_client::command::Reader::new("machine_sample:pipe");
        let parsed = horizon_client::command::block::parse(&mut reader, &game.ui.registries, false).unwrap();
        let state = game.block_state(&parsed).unwrap();
        let changed = game.native_set_block(dimension, [8,140,8], state, 3);
        eprintln!("BULK_TEST action={action:?} result={changed:?} entity={}", game.world.block_entities.get(&(8,140,8)).is_some());
        return;
    }
    let (name, filter, runs): (&str, Option<&str>, Vec<NativeBlockRun>) = match p[0] {
        "stone" | "glass" | "air" => {
            let block = match p[0] { "stone" => "minecraft:stone", "glass" => "minecraft:glass", _ => "minecraft:air" };
            let filter = (p[0] == "glass").then_some("minecraft:stone");
            let runs = (8..24).flat_map(|x| (8..24).map(move |z| NativeBlockRun { x, z, y: 140, height: 16, surface: None, block: 0, only: filter.map(|_| 0) })).collect();
            (block, filter, runs)
        }
        "one" => {
            let x = p[2].parse().unwrap(); let y = p[3].parse().unwrap(); let z = p[4].parse().unwrap();
            (p[1], None, vec![NativeBlockRun { x, z, y, height: 1, surface: None, block: 0, only: None }])
        }
        _ => return,
    };
    let edit = NativeBlockEdit { palette: vec![name.into()], filters: filter.into_iter().map(str::to_owned).collect(), runs };
    let result = game.native_edit_blocks(dimension, &edit);
    eprintln!("BULK_TEST action={action:?} result={result:?} entity={}", game.world.block_entities.get(&(8,140,8)).is_some());
}
