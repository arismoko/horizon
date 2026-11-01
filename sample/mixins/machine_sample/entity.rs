fn sample_entity(block: &str, entity: &mut Option<(String, Compound)>) {
    if entity.is_none() && (block == "machine_sample:machine" || block == "machine_sample:pipe") {
        static NEXT: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        eprintln!("ENTITY_TEST construct {block} serial={n}");
        let mut data = Compound::new();
        data.insert("serial".into(), horizon_world::nbt::Tag::Int(n));
        *entity = Some((block.to_owned(), data));
    }
}
