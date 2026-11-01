fn sample_bulk(game: &mut Game<'_>, command: &str, canceled: &mut bool) {
    if command == "say lightpacket" {
        *canceled = true;
        if let Some(c) = game.world.chunks.get_mut(&(2, 2)) {
            let c = std::sync::Arc::make_mut(c);
            let section = ((-16 - c.min_y()) / 16) as usize;
            std::sync::Arc::make_mut(&mut c.sections[section]).light = horizon_client::chunk::Light::Uniform(0xf0);
            game.world.bulk_log.insert((2, 2));
            eprintln!("BULK_LIGHT resend column=2,2 sky=15");
        }
        return;
    }
    if command == "say bulkinspect" {
        *canceled = true;
        eprintln!("BULK_INSPECT points={:?} surface={} block_log={:?}", [(2,-10,2),(15,-16,15),(16,-16,16),(63,-10,63)].map(|(x,y,z)| game.world.block(x,y,z).0), game.world.world_surface(32, 32, |s| s.0 == 0), game.world.block_log.as_ref().map(|l| l.len()));
        return;
    }
    if command != "say bulktest" { return; }
    *canceled = true;
    let chest = game.world.block(2, -10, 2).0;
    let runs: Vec<_> = (0..64).flat_map(|x| (0..64).map(move |z| horizon_client::block_edit::Run { x, z, y0: -32, y1: -3, state: 0, only: None })).collect();
    let result = game.world.edit_runs(&runs, |_, _| true, |s| s == chest && s != 0);
    eprintln!("BULK_TEST changed={} chunks={} tracked={:?} thread={:?}", result.changed, result.chunks, result.tracked, std::thread::current().name());
}
