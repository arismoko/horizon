fn sample_container(block: &str, container: &mut Option<ModContainer>) {
    if block == super::machine_sample::MACHINE && container.is_none() {
        *container = Some(ModContainer {
            slots: 9,
            faces: std::array::from_fn(|_| vec![0, 1]),
            inputs: vec![0],
            outputs: vec![1],
            menu: Some("minecraft:generic_9x1".into()),
        });
    }
}
