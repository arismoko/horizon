fn sample_entity(block: &str, entity: &mut Option<(String, Compound)>) {
    if entity.is_none() && (block == super::machine_sample::MACHINE) {
        *entity = Some((block.to_owned(), Compound::new()));
    }
}
