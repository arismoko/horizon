fn sample_fov(game: &Game<'_>, partial: f32, camera: [f64; 3], degrees: &mut f32) {
    static COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let active = game.stack_in(super::shield::OFFHAND_SLOT).is_some_and(|s| &*s.item == "minecraft:carrot_on_a_stick");
    let before = *degrees;
    if active { *degrees *= 0.5; }
    if COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 60 == 0 {
        eprintln!("FOV_TEST active={active} before={before} after={degrees} partial={partial} camera={camera:?} scoping={} scale={}", game.scoping(), game.fov_effect_scale());
    }
}
