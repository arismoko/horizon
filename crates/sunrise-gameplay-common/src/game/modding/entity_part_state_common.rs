/// Refuses each entity Part state declared on an entity type that keeps none: only the mob
/// types (`mob`, vanilla and mod) and the mods' general entity types do. Checked when the
/// mob types load, after every registration, since a type may be declared after its Part;
/// the error reads as the startup's other Part refusals (`part_admission`).
pub(in crate::game) fn refuse_entity_part_state_kinds(
    mob: impl Fn(&str) -> bool,
) -> Result<(), String> {
    let general = mod_api::__private::general_entity_type_declarations();
    let refusals: Vec<String> = mod_api::__private::entity_part_state_declarations()
        .iter()
        .filter_map(|declaration| {
            let reason = declaration.kind_refusal(
                &mob,
                |kind| general.iter().any(|declared| declared.id == kind),
                |kind| mod_api::__private::resolved_entity_type_key(kind).is_some(),
            )?;
            Some(format!(
                "{}: {}",
                declaration.mod_id,
                declaration.refused(reason)
            ))
        })
        .collect();
    if refusals.is_empty() {
        Ok(())
    } else {
        Err(format!("Mod parts refused:\n{}", refusals.join("\n")))
    }
}
