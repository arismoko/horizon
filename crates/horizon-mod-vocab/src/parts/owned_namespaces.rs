/// The package inventory carries the same namespace set as content files and
/// the package loader's collision fence. A package always owns its own ID.
pub fn owns_namespace(owner: &str, namespace: &str) -> bool {
    owner == namespace
        || crate::LoadedMods::new()
            .get(owner)
            .is_some_and(|package| package.namespaces().any(|owned| owned == namespace))
}

/// Validate and qualify a declaration using the package namespace inventory.
pub fn owned_declaration_id(owner: &str, name: &str) -> Result<String, String> {
    let id = if name.contains(':') {
        name.to_owned()
    } else {
        format!("{owner}:{name}")
    };
    let parsed = crate::Identifier::parse_namespaced(&id)
        .ok_or_else(|| format!("mod {owner} declares invalid identifier {id:?}"))?;
    let namespace = parsed.namespace();
    if !owns_namespace(owner, namespace) {
        return Err(format!(
            "mod {owner} declares {id:?} in unowned namespace {namespace:?}"
        ));
    }
    Ok(id)
}

impl crate::RegistrationContext {
    /// Refuse this mod's registration with a diagnostic, without panicking.
    /// The first reason wins. Reasons exceeding 4096 UTF-8 bytes are truncated
    /// at a character boundary; shorter reasons are preserved verbatim.
    /// All this mod's declarations from this registration are discarded, including
    /// later declarations, without adding their validation errors to the report.
    /// Other mods' declarations are unchanged. Native and wasm use the same
    /// admission path. Returning from `Hooks::register` immediately is recommended.
    pub fn refuse(&self, reason: &str) {
        let mut reason_slot = self.self_refusal.lock().unwrap_or_else(|p| p.into_inner());
        if reason_slot.is_none() {
            let mut end = reason.len().min(4096);
            while !reason.is_char_boundary(end) {
                end -= 1;
            }
            *reason_slot = Some(reason[..end].to_owned());
        }
    }

    pub(crate) fn refuse_declaration(&self, reason: String) {
        self.declaration_refusals
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(reason);
    }
}

/// The engine consumes block/item declaration ownership refusals before loading
/// resources. Guest dispatch retains invalid output so the owning host boundary
/// sees and refuses it as well, rather than silently dropping a declaration.
// hidden: engine admission plumbing, re-exported through __private for the loader.
#[doc(hidden)]
pub fn take_declaration_refusals(context: &crate::RegistrationContext) -> Vec<String> {
    std::mem::take(
        &mut *context
            .declaration_refusals
            .lock()
            .unwrap_or_else(|p| p.into_inner()),
    )
}
