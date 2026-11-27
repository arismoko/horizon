use super::*;

/// Collect the mods' special item model factories once at client startup, before loading
/// resources (NeoForge's `RegisterSpecialModelRendererEvent`). The last registration of a
/// type wins over any earlier mod or builtin registration. Resource loads reuse the factories
/// to decode and bake each `minecraft:special` node, rather than firing this hook again.
pub fn native_register_special_model_renderer(
    context: &mod_api::RegistrationContext,
    renderers: &mut mod_api::SpecialModelRendererRegistry,
) {
    crate::game::native_hooks::fire::fire_register_special_model_renderer(context, |listen| {
        renderers.register_listener(listen)
    });
}

#[cfg(test)]
mod tests {
    use mod_api::__private::game::RegistrationContextNew;
    use mod_api::{
        ClientHost, Hooks, ModEntry, RegisterSpecialModelRendererEvent, RegistrationContext,
        SpecialModelBakeContext, SpecialModelCollector, SpecialModelConfig, SpecialModelContext,
        SpecialModelFactory, SpecialModelGeometry, SpecialModelQuad, SpecialModelRenderer,
        SpecialModelRendererRegistry, SpecialModelStack,
    };

    struct NoGeometry;
    impl SpecialModelBakeContext for NoGeometry {
        fn bake_model(&self, _: &str) -> Result<SpecialModelGeometry, String> {
            unreachable!("fixture uses only the decoder")
        }

        fn bake_geometry(&self, _: &[SpecialModelQuad]) -> Result<SpecialModelGeometry, String> {
            unreachable!("fixture uses only the decoder")
        }
    }

    struct NoDraw;
    impl SpecialModelRenderer for NoDraw {
        type Argument = ();

        fn extract_argument(&self, _: &dyn ClientHost, _: &SpecialModelStack<'_>) {}

        fn submit(
            &self,
            _: &(),
            _: &SpecialModelContext,
            _: &mut SpecialModelCollector<'_>,
        ) -> Result<(), String> {
            unreachable!("fixture uses only the decoder")
        }
    }

    struct Registrant {
        rows: &'static [(&'static str, &'static str)],
        panics: bool,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Registrant {
        fn register_special_model_renderer(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterSpecialModelRendererEvent<'_>,
        ) {
            for &(kind, marker) in self.rows {
                event.renderers.register(
                    mod_api::Identifier::parse(kind).unwrap(),
                    SpecialModelFactory::new(move |_| Err(marker.into()), |(), _| Ok(NoDraw)),
                );
            }
            if self.panics {
                panic!("special model registrar fixture failed");
            }
        }
    }

    #[test]
    fn a_panicking_registration_leaves_the_winning_factory_and_reports_unchanged() {
        mod_platform::testing::with_mods(
            vec![
                ModEntry::new(
                    "first",
                    Box::new(Registrant {
                        rows: &[("minecraft:shield", "first")],
                        panics: false,
                    }),
                ),
                ModEntry::new(
                    "winner",
                    Box::new(Registrant {
                        rows: &[
                            ("minecraft:shield", "intermediate"),
                            ("test:pan", "pan"),
                            ("minecraft:shield", "winner"),
                        ],
                        panics: false,
                    }),
                ),
                ModEntry::new(
                    "failed",
                    Box::new(Registrant {
                        rows: &[
                            ("minecraft:shield", "failed"),
                            ("test:leaked", "leaked"),
                            ("minecraft:shield", "failed again"),
                        ],
                        panics: true,
                    }),
                ),
            ]
            .into(),
            || {
                let mut registry = SpecialModelRendererRegistry::default();
                crate::game::native_hooks::native_register_special_model_renderer(
                    &RegistrationContext::new(),
                    &mut registry,
                );
                let shield = mod_api::Identifier::parse("minecraft:shield").unwrap();
                let pan = mod_api::Identifier::parse("test:pan").unwrap();
                assert_eq!(
                    registry.initial_owners().collect::<Vec<_>>(),
                    [(&shield, Some("first")), (&pan, Some("winner"))],
                );
                let (factories, reports) = registry.into_renderers();
                assert_eq!(factories.len(), 2);
                let fields = mod_api::serde_json::Map::new();
                assert_eq!(
                    factories[&shield]
                        .1
                        .bake(SpecialModelConfig::new(&fields), &NoGeometry)
                        .err()
                        .as_deref(),
                    Some("decode: winner"),
                );
                assert_eq!(
                    reports
                        .iter()
                        .map(|entry| (entry.kind.clone(), entry.earlier, entry.later))
                        .collect::<Vec<_>>(),
                    [
                        (shield.clone(), Some("first"), Some("winner")),
                        (shield, Some("winner"), Some("winner")),
                    ],
                );
            },
        );
    }
}
