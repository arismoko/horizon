use sunrise_level::effects::EffectInstance;

/// `LivingEntity.forceAddEffect` with the packet's `shouldBlend` (flag 8): the effect replaces
/// any of its kind, an update keeping its fade (`copyBlendState`). A new one not blended starts
/// at its target (`skipBlending`), except the kinds in `beside`, whose fade the caller keeps
/// beside the list. Returns the instance when it is new, for those fades.
pub fn force_add_effect<'a>(
    list: &'a mut Vec<EffectInstance>,
    mut e: EffectInstance,
    blend: bool,
    beside: &[&str],
) -> Option<&'a EffectInstance> {
    match list.iter().position(|x| x.id == e.id) {
        Some(i) => {
            e.client_blend = list[i].client_blend;
            list[i] = e;
            None
        }
        None => {
            if !blend && !beside.contains(&e.id) {
                let has_effect = !e.ends_within(0);
                e.client_blend = (has_effect, has_effect);
            }
            list.push(e);
            list.last()
        }
    }
}
