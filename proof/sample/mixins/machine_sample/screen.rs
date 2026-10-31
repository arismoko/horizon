fn sample_screen(block: &str, screen: &mut Option<NativeScreen>) {
    if block == super::machine_sample::MACHINE && screen.is_none() {
        *screen = Some(NativeScreen {
            width: 176.0,
            height: 166.0,
            slots: vec![Some([44.0, 35.0]), Some([116.0, 35.0])],
            inventory_x: 8.0,
            inventory_y: 84.0,
            own_labels: false,
        });
    }
}
