fn sample_entity(block: &str, entity: &mut Option<(String, Compound)>) {
    if entity.is_none() && (block == "machine_sample:machine" || block == "machine_sample:pipe") {
        *entity = Some((block.to_owned(), Compound::new()));
    }
}
