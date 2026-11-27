#[cfg(test)]
mod tests {
    use mod_api::{__private::game::StandaloneModelRegistration, RegistrationContext};
    use sunrise_gameplay_common::game::{
        execution_role::ExecutionRole, mods::standalone::collect_standalone_models,
    };

    #[test]
    fn standalone_models_collect_only_on_the_client() {
        use mod_api::{self, Hooks, ModEntry, StandaloneModelsEvent};
        use mod_platform::testing::with_mods;
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        #[derive(Default)]
        struct Models(Arc<AtomicUsize>);
        #[mod_api::sunrise_mod]
        impl Hooks for Models {
            fn standalone_models(
                &self,
                context: &RegistrationContext,
                event: &mut StandaloneModelsEvent<'_>,
            ) {
                assert_eq!(
                    context.mods().ids().collect::<Vec<_>>(),
                    ["registration_test"]
                );
                self.0.fetch_add(1, Ordering::Relaxed);
                let shaft = "registration_test:block/shaft";
                event.register_model(shaft);
                event.register("other_namespace:shaft_east", [shaft], move |bake| {
                    bake.append_dependency(shaft, [0, 1, 0], true)
                });
            }
        }
        let calls = Arc::new(AtomicUsize::new(0));
        with_mods(
            vec![ModEntry::new(
                "registration_test",
                Box::new(Models(calls.clone())),
            )]
            .into(),
            || {
                mod_platform::__private::with_test_packages(&["registration_test"], || {
                    assert!(collect_standalone_models(ExecutionRole::IntegratedServer).is_none());
                    assert!(collect_standalone_models(ExecutionRole::DedicatedServer).is_none());
                    assert_eq!(calls.load(Ordering::Relaxed), 0);
                    let registrations = collect_standalone_models(ExecutionRole::Client).unwrap();
                    assert_eq!(calls.load(Ordering::Relaxed), 1);
                    let [plain, transformed] = &registrations[..] else {
                        panic!("expected a file followed by a baker");
                    };
                    assert!(matches!(
                        &plain.answer,
                        StandaloneModelRegistration::File(id) if id == "registration_test:block/shaft"
                    ));
                    let StandaloneModelRegistration::Baker(baker) = &transformed.answer else {
                        panic!("expected a baker");
                    };
                    assert_eq!(transformed.mod_id, "registration_test");
                    assert_eq!(baker.id, "other_namespace:shaft_east");
                    assert_eq!(baker.dependencies, ["registration_test:block/shaft"]);
                })
            },
        );
    }
}
