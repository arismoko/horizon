//! Content sound slots retain dynamic vanilla selection when inherited.
use std::borrow::Cow;
use sunrise_mod_package::content::entity_sounds::{
    EntitySounds, SoundOverride, StepOverride, StepSound,
};

#[derive(Clone, Copy, Debug, Default)]
pub enum Selection {
    #[default]
    Inherit,
    Id(&'static str),
    Mute,
}
impl Selection {
    /// No lookup or allocation for inherited or muted slots.
    pub fn select(self, inherited: &'static str) -> &'static str {
        match self {
            Self::Inherit => inherited,
            Self::Id(id) => id,
            Self::Mute => "",
        }
    }
}
#[derive(Clone, Debug, Default)]
pub struct Sounds {
    pub ambient: Selection,
    pub hurt: Selection,
    pub death: Selection,
    pub step: Selection,
    /// A declared step's `playSound` volume and pitch; a plain id plays at the
    /// block step's 0.15 and 1.0.
    pub step_volume_pitch: Option<(f32, f32)>,
    /// `LivingEntity.getSoundVolume`, in place of the template's.
    pub volume: Option<f32>,
    /// The base of `LivingEntity.getVoicePitch`, in place of 1.0.
    pub voice_pitch: Option<f32>,
    /// `Mob.getAmbientSoundInterval`, read once into the type's `ambient_interval`.
    pub ambient_interval: Option<i32>,
}
impl Sounds {
    pub fn load(entity: &str, row: &serde_json::Value) -> crate::Result<Self> {
        let sounds: EntitySounds = match row.get("sounds") {
            None => EntitySounds::default(),
            Some(value) => serde_json::from_value(value.clone())
                .map_err(|error| format!("{entity}: sounds: {error}"))?,
        };
        sounds.validate(entity)?;
        let resolve = |slot| match slot {
            None => Selection::Inherit,
            Some(SoundOverride::Mute(_)) => Selection::Mute,
            Some(SoundOverride::Id(id)) => Selection::Id(intern_full_id(&id)),
        };
        let (step, step_volume_pitch) = match sounds.step {
            None => (Selection::Inherit, None),
            Some(StepOverride::Mute(_)) => (Selection::Mute, None),
            Some(StepOverride::Id(id)) => (Selection::Id(intern_full_id(&id)), None),
            Some(StepOverride::Sound(StepSound {
                sound,
                volume,
                pitch,
            })) => (Selection::Id(intern_full_id(&sound)), Some((volume, pitch))),
        };
        Ok(Self {
            ambient: resolve(sounds.ambient),
            hurt: resolve(sounds.hurt),
            death: resolve(sounds.death),
            step,
            step_volume_pitch,
            volume: sounds.volume,
            voice_pitch: sounds.voice_pitch,
            ambient_interval: sounds.ambient_interval,
        })
    }
    /// The base `getVoicePitch` spreads around: the declared one, else 1.0.
    pub fn voice_pitch(&self) -> f32 {
        self.voice_pitch.unwrap_or(1.0)
    }
}
/// Namespace normalization at audio/network boundaries. Full ids are borrowed unchanged.
pub fn full_id(id: &str) -> Cow<'_, str> {
    if id.contains(':') {
        Cow::Borrowed(id)
    } else {
        Cow::Owned(format!("minecraft:{id}"))
    }
}

pub(crate) fn intern_full_id(id: &str) -> &'static str {
    use std::{collections::HashSet, sync::Mutex};
    static IDS: Mutex<Option<HashSet<&'static str>>> = Mutex::new(None);
    let mut ids = IDS.lock().unwrap_or_else(|e| e.into_inner());
    let ids = ids.get_or_insert_with(HashSet::new);
    if let Some(id) = ids.get(id) {
        return id;
    }
    let id = Box::leak(id.to_owned().into_boxed_str());
    ids.insert(id);
    id
}

pub fn ambient(m: &mut super::Mob, w: &super::MobWorld, events: &mut Vec<super::MobEvent>) {
    match m.kind.sound_overrides.ambient {
        Selection::Inherit => super::ai::play_ambient(m, w, events),
        Selection::Id(id) => m.voice(id, events),
        Selection::Mute => {}
    }
}
pub fn voices(m: &super::Mob) -> [&'static str; 3] {
    let inherited = super::ai::inherited_voice_sounds(m);
    let slots = &m.kind.sound_overrides;
    [
        slots.ambient.select(inherited[0]),
        slots.hurt.select(inherited[1]),
        slots.death.select(inherited[2]),
    ]
}
pub fn step(
    m: &super::Mob,
    w: &super::MobWorld,
    pos: [i32; 3],
    state: sunrise_world::registry::BlockStateId,
) -> Option<(Cow<'static, str>, f32, f32)> {
    declared_step(&m.kind.sound_overrides)
        .unwrap_or_else(|| super::ai::inherited_step_sound(m, w, pos, state))
}
/// The step a content type declares (`playSound(step, volume, pitch)`, or
/// silence), or `None` when the slot inherits the template's.
pub fn declared_step(sounds: &Sounds) -> Option<Option<(Cow<'static, str>, f32, f32)>> {
    match sounds.step {
        Selection::Inherit => None,
        Selection::Id(id) => {
            let (volume, pitch) = sounds.step_volume_pitch.unwrap_or((0.15, 1.0));
            Some(Some((Cow::Borrowed(id), volume, pitch)))
        }
        Selection::Mute => Some(None),
    }
}
