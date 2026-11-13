use super::*;
use mod_api::__private::game::RegistrationContextNew;
use mod_api::{
    BlockEntityDrawContext, EntityRenderState, EntityRenderer, EntityRendererRegistry,
    EntityRenderers,
};
/// Lights what it draws `base` plus the entity's id, after moving up by its age.
struct Marked(u32);
impl EntityRenderer for Marked {
    fn submit(
        &self,
        entity: &EntityRenderState,
        draw: &mut dyn BlockEntityDrawContext,
    ) -> Result<(), String> {
        draw.translate([0.0, entity.age, 0.0]);
        draw.rotate_about([0.0, 1.0, 0.0], -entity.yaw, [0.0; 3]);
        draw.set_light(self.0 + entity.id);
        Ok(())
    }
}
pub(super) fn register(renderers: &mut dyn EntityRenderers) {
    renderers.register("wire_test:bolt".into(), Arc::new(Marked(100)));
    renderers.register("wire_test:dart".into(), Arc::new(Marked(200)));
    // The later registration for a type wins.
    renderers.register("wire_test:bolt".into(), Arc::new(Marked(300)));
}
struct Native;
#[mod_api::__subscriptions]
impl Hooks for Native {
    fn register_entity_renderers(
        &self,
        _: &mod_api::RegistrationContext,
        event: &mut mod_api::RegisterEntityRenderersEvent<'_>,
    ) {
        register(event.renderers);
    }
}
/// What a renderer did to the draw it was handed.
#[derive(Default, Debug, PartialEq)]
struct Draw {
    moved: Vec<[f32; 3]>,
    turned: Vec<([f32; 3], f32)>,
    light: u32,
}
impl BlockEntityDrawContext for Draw {
    fn translate(&mut self, by: [f32; 3]) {
        self.moved.push(by);
    }
    fn rotate_about(&mut self, axis: [f32; 3], degrees: f32, _: [f32; 3]) {
        self.turned.push((axis, degrees));
    }
    fn set_light(&mut self, light: u32) {
        self.light = light;
    }
}
#[test]
fn wire_loopback_entity_renderers_register_and_draw_alike_for_native_and_guest_mods() {
    for guest in [false, true] {
        let hooks: Box<dyn Hooks> = if guest {
            Box::new(WasmMod {
                group: 0,
                subscribed: &[Some(mod_api::HookId::RegisterEntityRenderers)],
                listeners: &[],
                corrupt: false,
            })
        } else {
            Box::new(Native)
        };
        mod_api::testing::with_mods(
            vec![mod_api::ModEntry::new("wire_test", hooks)].into(),
            || {
                let mut registry = EntityRendererRegistry::default();
                mod_api::dispatch_register_entity_renderers(
                    &mod_api::RegistrationContext::new(),
                    &mut registry,
                );
                let renderers = registry.into_renderers();
                assert_eq!(
                    renderers.keys().collect::<Vec<_>>(),
                    ["wire_test:bolt", "wire_test:dart"],
                    "guest {guest}"
                );
                let entity: EntityRenderState = mod_api::__private::built::EntityRenderState {
                    id: 7,
                    partial: 0.25,
                    age: 12.25,
                    yaw: 90.0,
                    pitch: -10.0,
                }
                .into();
                for (kind, light) in [("wire_test:bolt", 307), ("wire_test:dart", 207)] {
                    let mut draw = Draw::default();
                    assert_eq!(renderers[kind].submit(&entity, &mut draw), Ok(()));
                    assert_eq!(
                        draw,
                        Draw {
                            moved: vec![[0.0, 12.25, 0.0]],
                            turned: vec![([0.0, 1.0, 0.0], -90.0)],
                            light,
                        },
                        "{kind}, guest {guest}"
                    );
                }
                if guest {
                    // A guest call whose reply doesn't decode (as a refused or trapped call
                    // leaves none) is an `Err`, never a silent `Ok`: the game then draws the
                    // type's own geometry.
                    APPEND_ATTACK.set(3);
                    let refused = renderers["wire_test:bolt"].submit(&entity, &mut Draw::default());
                    APPEND_ATTACK.set(0);
                    assert!(
                        refused
                            .as_ref()
                            .is_err_and(|e| e.starts_with("wasm call refused or trapped")),
                        "{refused:?}"
                    );
                }
            },
        );
    }
}
