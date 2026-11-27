//! The general entity renderer used by the integrated scene.
use mod_api::{
    AttachmentCodec, AttachmentDescriptor, AttachmentError, AttachmentKey, BlockEntityDrawContext,
    EntityRenderState, EntityRenderer, EntityRenderers, GeneralEntityCategory, GeneralSavePolicy,
    Registrar, Tag,
};
use std::sync::Mutex;

pub const KIND: &str = "general_render_fixture:render_probe";

const FUEL_CODEC: AttachmentCodec<i32> = AttachmentCodec {
    encode: |n| Ok([("n".into(), Tag::Int(*n))].into()),
    decode: |tag| match tag.get(&"n".into()) {
        Some(Tag::Int(n)) => Ok(*n),
        _ => Err(AttachmentError::Decode),
    },
};

#[derive(Default)]
pub struct Keys {
    fuel: Mutex<Option<AttachmentKey<i32>>>,
}

impl Keys {
    pub fn fuel(&self) -> AttachmentKey<i32> {
        self.fuel
            .lock()
            .unwrap()
            .clone()
            .expect("general_render_fixture:render_fuel registered")
    }
}

pub(crate) fn register(registrar: &mut dyn Registrar, keys: &Keys) {
    let fuel = registrar
        .register_attachment_type(
            AttachmentDescriptor::new("general_render_fixture:render_fuel", || 0i32)
                .serialize(FUEL_CODEC)
                .sync(FUEL_CODEC),
        )
        .unwrap();
    *keys.fuel.lock().unwrap() = Some(fuel);
    registrar
        .declare_general_entity(
            "render_probe",
            GeneralEntityCategory::Misc,
            0.5,
            0.5,
            None,
            GeneralSavePolicy::Save,
            true,
            false,
            false,
            None,
            8,
            1,
            1,
            &[],
        )
        .unwrap();
}

pub(crate) fn register_renderer(renderers: &mut dyn EntityRenderers, keys: &Keys) {
    renderers.register(
        KIND.into(),
        std::sync::Arc::new(Probe { fuel: keys.fuel() }),
    );
}

struct Probe {
    fuel: AttachmentKey<i32>,
}
impl EntityRenderer for Probe {
    fn submit(
        &self,
        entity: &EntityRenderState,
        draw: &mut dyn BlockEntityDrawContext,
    ) -> Result<(), String> {
        let fuel = entity
            .attachments
            .as_ref()
            .ok_or("missing render snapshot")?
            .get(&self.fuel)
            .map_err(|e| format!("{e:?}"))?
            .copied()
            .ok_or("missing fuel")? as f32;
        draw.quad(
            "block/stone",
            [
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0],
                [fuel, 0.0, 0.0],
                [fuel, 1.0, 0.0],
            ],
            [0.0, 0.0, 16.0, 16.0],
            0xFF_FFFF,
        );
        Ok(())
    }
}
