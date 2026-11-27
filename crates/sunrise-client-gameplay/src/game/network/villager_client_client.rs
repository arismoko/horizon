use std::sync::Arc;
use sunrise_protocol::{entity_data::Value, registry::Builtin};
use sunrise_sim::mobs::villager::ResolvedProfessions;
use sunrise_sim::mobs::{Mob, Species};

impl crate::game::ClientGame<'_> {
    pub(in crate::game::client_side) fn client_professions(
        &self,
    ) -> super::Result<Arc<ResolvedProfessions>> {
        let table = self.prepare_client_professions()?;
        self.install_client_professions(&table);
        Ok(table)
    }
    pub(in crate::game) fn install_client_professions(&self, table: &ResolvedProfessions) {
        sunrise_client::mobs::render::villager_client::install(
            table,
            &self
                .client_res()
                .client_mob_data()
                .villager_hats
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
    }
    pub(in crate::game) fn prepare_client_professions(
        &self,
    ) -> super::Result<Arc<ResolvedProfessions>> {
        use mod_api::__private::game::RegistrationContextNew;
        let declarations = self.common.mods.profession_declarations.get_or_init(|| {
            let mut registrar = mod_api::ProfessionDeclarations::default();
            sunrise_gameplay_common::game::native_hooks::native_register_villager_professions(
                &mod_api::RegistrationContext::new(),
                &mut registrar,
            );
            registrar.into_entries()
        });
        // The client consumes only identity slots. World-dependent POI/trade references
        // are validated by the authoritative server's complete world preparation.
        ResolvedProfessions::prepare(declarations.clone(), |_, _| true)
            .map(Arc::new)
            .map_err(|errors| format!("client villager professions refused: {errors:?}").into())
    }
}

/// Apply the ordinary metadata, then resolve profession slots in the owning load.
pub(in crate::game) fn receive(
    m: &mut Mob,
    values: &[(u8, i32, Value)],
    builtin: &Builtin,
    entity: i32,
    table: &ResolvedProfessions,
) -> Option<[f64; 3]> {
    let sound = super::remote_mob_data::apply(m, values, builtin, entity);
    let index = match m.species() {
        Species::Villager => 19,
        Species::ZombieVillager => 20,
        _ => return sound,
    };
    for (field, _, value) in values {
        if *field == index
            && let Value::Ids(data) = value
            && let [_, profession, _] = data.as_slice()
        {
            m.extra.profession = usize::try_from(*profession)
                .ok()
                .and_then(|slot| table.state(slot))
                .unwrap_or_default();
        }
    }
    sound
}
