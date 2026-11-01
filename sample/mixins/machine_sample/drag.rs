fn sample_drag(screen: &NativeScreenView<'_>, button: i32, delta: [f32; 2], canceled: &mut bool) {
    *canceled = std::path::Path::new("/box/work/sample-5905/cancel-drag").exists();
    eprintln!("DRAG_TEST screen={} button={button} mouse={:?} delta={delta:?} cancel={canceled}", screen.id, screen.mouse);
}
