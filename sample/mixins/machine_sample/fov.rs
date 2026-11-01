fn sample_fov(game: &Game<'_>, partial: f32, camera: [f64; 3], degrees: &mut f32) {
    static COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let item = game.stack_in(super::shield::OFFHAND_SLOT).map(|s| &*s.item);
    let factor = match item { Some("minecraft:carrot_on_a_stick") => 0.5, Some("minecraft:warped_fungus_on_a_stick") => 0.05, _ => 1.0 };
    let before = *degrees;
    *degrees *= factor;
    if COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 60 == 0 {
        eprintln!("FOV_TEST factor={factor} before={before} after={degrees} partial={partial} camera={camera:?} scoping={} scale={}", game.scoping(), game.fov_effect_scale());
    }
}
