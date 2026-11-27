//! A living entity's absorption amount read for native mods (`LivingEntity.getAbsorptionAmount`).
use mod_api::LivingRef;

pub trait NativeLivingAbsorptionGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// The entity's absorption amount (`LivingEntity.getAbsorptionAmount`).
    fn living_absorption(&self, living: LivingRef) -> Option<f32> {
        self.side_living_absorption(living)
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> NativeLivingAbsorptionGameExt1<'__sg> for G {}

use crate::game::side_ops::modapi_modding::ModapiModdingSideGameExt as _;
