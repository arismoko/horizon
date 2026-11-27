use super::registration_registry_names;
use crate::game::part_admission::admit_parts;
use mod_api::{
    Dist, EffectKey, Hooks, InstanceDirectory, MobEffectPart, MobEffectPartError, ModEntry,
    Registrar, RegistrationContext, RuntimeEnvironment,
};
use mod_platform::{__private::with_test_registration_data, testing::with_mods};
use std::fs;

struct EffectPart;
#[mod_api::sunrise_part]
impl MobEffectPart for EffectPart {}

#[test]
fn a_builtin_effect_missing_from_the_prepared_report_still_refuses_a_part() {
    type Outcome =
        std::sync::Arc<std::sync::Mutex<Option<(EffectKey, Result<(), MobEffectPartError>)>>>;
    struct Attaches(Outcome);
    #[mod_api::__subscriptions]
    impl Hooks for Attaches {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let effect = r.effect_key("minecraft:speed");
            let result = r.mob_effect_part(&effect, EffectPart).map(|_| ());
            *self.0.lock().unwrap() = Some((effect, result));
        }
    }
    let outcome = Outcome::default();
    let fixture = tempfile::tempdir().unwrap();
    fs::write(
        fixture.path().join("registries.json"),
        r#"{"minecraft:mob_effect":{"entries":{"minecraft:slowness":{"protocol_id":1}}}}"#,
    )
    .unwrap();
    let data = mod_api::__private::RegistrationData::new(
        registration_registry_names(fixture.path()).unwrap(),
        Default::default(),
        InstanceDirectory::from_path(fixture.path()).unwrap(),
        RuntimeEnvironment::new(Dist::Client, "26.3".into(), "test".into(), false),
    );
    with_test_registration_data(data, || {
        let mods: mod_api::Mods =
            vec![ModEntry::new("addon", Box::new(Attaches(outcome.clone())))].into();
        with_mods(mods.clone(), || admit_parts(&mods).unwrap());
    });
    let (effect, result) = outcome.lock().unwrap().take().expect("the callback ran");
    assert_eq!(result, Err(MobEffectPartError::VanillaEffect { effect }));
}
