//! The existing living-health clocks plus Sunrise's cancelled-die continuation.
use super::EntityTagWriter;
use crate::{mobs::Mob, save::number};
use sunrise_core::nbt::Tag;

pub(super) fn write(mob: &Mob, out: &mut EntityTagWriter) {
    out.insert("Health".into(), Tag::Float(mob.health.max(0.0).to_bits()));
    // LivingEntity.addAdditionalSaveData: (short) casts, nothing clamped.
    out.insert("HurtTime".into(), Tag::Short(mob.hurt_time as i16));
    out.insert("DeathTime".into(), Tag::Short(mob.death_time as i16));
    if !mob.death_canceled {
        out.remove(&"sunrise:death_canceled".into());
    }
}

pub(super) fn read(mob: &mut Mob, tag: &Tag) {
    mob.death_time = i32::from(number(tag.get("DeathTime")).unwrap_or(0) as i16);
    mob.death_canceled = super::flag(tag, "sunrise:death_canceled");
}
