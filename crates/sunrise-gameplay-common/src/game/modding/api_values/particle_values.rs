//! Particle keys cross the game boundary through the load-resolved provider table.
use super::{TryIntoApi, TryIntoEngine};
use mod_api::{__private::ParticleKind, ParticleOptions as ApiOptions};
use {
    sunrise_level::particle_options::ParticleOptions as EngineOptions,
    sunrise_level::particle_options::ParticleType,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParticleValueError {
    UnknownProvider,
    NonFinite,
    /// An item crumb's colour has a component outside `0.0..=1.0`.
    ColorRange,
    InvalidStack,
}

fn finite(options: &ApiOptions) -> bool {
    options.color.into_iter().all(f32::is_finite)
        && options.to_color.into_iter().all(f32::is_finite)
        && options.scale.is_finite()
        && options
            .trail
            .as_ref()
            .is_none_or(|(at, _)| at.iter().all(|v| v.is_finite()))
        && options
            .geyser
            .is_none_or(|(_, impulse)| impulse.is_finite())
        && options.vibration.as_ref().is_none_or(|(source, at, _)| {
            at.iter().all(|v| v.is_finite())
                && match source {
                    mod_api::client::particles::PositionSource::Block(_) => true,
                    mod_api::client::particles::PositionSource::Entity { y_offset, .. } => {
                        y_offset.is_finite()
                    }
                    _ => false,
                }
        })
}
pub(crate) trait ParticleItems {
    fn particle_items(&self) -> Option<&sunrise_items::items::Items>;
}
impl ParticleItems for () {
    fn particle_items(&self) -> Option<&sunrise_items::items::Items> {
        None
    }
}
impl ParticleItems for sunrise_items::items::Items {
    fn particle_items(&self) -> Option<&Self> {
        Some(self)
    }
}
impl<C: ParticleItems + ?Sized> TryIntoEngine<C> for ApiOptions {
    type Engine = EngineOptions;
    type Error = ParticleValueError;
    fn try_into_engine(&self, context: &C) -> Result<Self::Engine, Self::Error> {
        if let Some(value) = self.typed() {
            return EngineOptions::typed(value.clone()).ok_or(ParticleValueError::UnknownProvider);
        }
        if mod_api::__private::particle_override_limiter(self.key()).is_some() {
            return Err(ParticleValueError::UnknownProvider);
        }
        if !finite(self) {
            return Err(ParticleValueError::NonFinite);
        }
        let (ty, custom) = match mod_api::__private::particle_kind(self.key())
            .ok_or(ParticleValueError::UnknownProvider)?
        {
            ParticleKind::Vanilla(index) => (
                *ParticleType::ALL
                    .get(index as usize)
                    .ok_or(ParticleValueError::UnknownProvider)?,
                None,
            ),
            ParticleKind::Custom => (
                ParticleType::Smoke,
                Some(std::borrow::Cow::Borrowed(
                    mod_api::__private::particle_name(self.key())
                        .ok_or(ParticleValueError::UnknownProvider)?,
                )),
            ),
        };
        if ty.is_item_crumb() && !self.color.into_iter().all(|c| (0.0..=1.0).contains(&c)) {
            return Err(ParticleValueError::ColorRange);
        }
        Ok(EngineOptions {
            polytone_seed: None,
            source: None,
            ty,
            mod_particle: custom,
            typed_particle: None,
            block: self.block.clone(),
            color: self.color,
            scale: self.scale,
            to_color: self.to_color,
            trail: self.trail,
            item: self.item.clone(),
            item_stack: self
                .item_stack
                .as_ref()
                .map(|stack| {
                    stack
                        .try_into_engine(
                            context
                                .particle_items()
                                .ok_or(ParticleValueError::InvalidStack)?,
                        )
                        .map_err(|_| ParticleValueError::InvalidStack)
                })
                .transpose()?,
            geyser: self.geyser,
            vibration: self.vibration.clone(),
        })
    }
}
impl TryIntoApi for EngineOptions {
    type Api = ApiOptions;
    type Error = ParticleValueError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        if let Some((value, _)) = &self.typed_particle {
            return Ok(ApiOptions::from_typed(value.clone()));
        }
        let key = match self.mod_particle.as_deref() {
            Some(name) => mod_api::__private::particle_named_key(name),
            None => mod_api::__private::particle_vanilla_key(self.ty as u32),
        }
        .ok_or(ParticleValueError::UnknownProvider)?;
        let mut options = ApiOptions::simple(key);
        options.block = self.block.clone();
        options.color = self.color;
        options.scale = self.scale;
        options.to_color = self.to_color;
        options.trail = self.trail;
        options.item = self.item.clone();
        options.item_stack = self
            .item_stack
            .as_ref()
            .map(|stack| {
                stack
                    .try_into_api()
                    .map_err(|_| ParticleValueError::InvalidStack)
            })
            .transpose()?;
        options.geyser = self.geyser;
        options.vibration = self.vibration.clone();
        Ok(options)
    }
}
impl TryIntoEngine<()> for mod_api::FireworkExplosion {
    type Engine = sunrise_items::firework::FireworkExplosion;
    type Error = std::convert::Infallible;
    fn try_into_engine(&self, context: &()) -> Result<Self::Engine, Self::Error> {
        Ok(Self::Engine {
            shape: self.shape.try_into_engine(context)?,
            colors: self.colors.clone(),
            fade: self.fade_colors.clone(),
            trail: self.trail,
            twinkle: self.twinkle,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::client::particles::PositionSource;

    fn prepare() {
        mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
        mod_api::__private::resolve_particles(
            [
                "minecraft:smoke",
                "minecraft:trail",
                "minecraft:item",
                "test:spark",
            ],
            |id| {
                if id == "test:spark" {
                    Some(ParticleKind::Custom)
                } else {
                    ParticleType::from_name(id).map(|ty| ParticleKind::Vanilla(ty as u32))
                }
            },
        )
        .unwrap();
    }
    #[test]
    fn particle_keys_preserve_provider_and_every_option_slot() {
        prepare();
        let mut original = EngineOptions::trail([3.0, 4.0, 5.0], 0x112233, 19);
        original.scale = 2.5;
        original.to_color = [0.1, 0.3, 0.7];
        original.block = Some(("test:sprite".into(), 0x123456));
        original.item = Some(vec!["test:layer".into()]);
        original.vibration = Some((
            mod_api::client::particles::PositionSource::Entity {
                id: -7,
                y_offset: 0.5,
            },
            [1.0, 2.0, 3.0],
            11,
        ));
        let api = original.try_into_api().unwrap();
        assert_eq!(
            mod_api::__private::particle_kind(api.key()),
            Some(ParticleKind::Vanilla(ParticleType::Trail as u32))
        );
        let result = api.try_into_engine(&()).unwrap();
        assert_eq!(result.ty, ParticleType::Trail);
        assert_eq!(result.trail, Some(([3.0, 4.0, 5.0], 19)));
        assert_eq!(
            result.color,
            [17.0 / 255.0, 34.0 / 255.0, 51.0 / 255.0, 1.0]
        );
        assert_eq!(result.scale, 2.5);
        assert_eq!(result.to_color, [0.1, 0.3, 0.7]);
        assert_eq!(result.block, Some(("test:sprite".into(), 0x123456)));
        assert_eq!(result.item, Some(vec!["test:layer".into()]));
        assert_eq!(
            result.vibration,
            Some((
                mod_api::client::particles::PositionSource::Entity {
                    id: -7,
                    y_offset: 0.5
                },
                [1.0, 2.0, 3.0],
                11
            ))
        );
    }
    #[test]
    fn particle_geyser_options_cross_the_boundary() {
        prepare();
        let value = EngineOptions::geyser_base(ParticleType::Smoke, 7, 1.25)
            .try_into_api()
            .unwrap();
        assert_eq!(value.try_into_engine(&()).unwrap().geyser, Some((7, 1.25)));
    }
    #[test]
    fn particle_custom_provider_borrows_its_retained_name() {
        prepare();
        let value = EngineOptions::named("test:spark").try_into_api().unwrap();
        let published = mod_api::__private::particle_name(value.key()).unwrap();
        let output = value.try_into_engine(&()).unwrap();
        assert_eq!(output.mod_particle.as_deref(), Some("test:spark"));
        assert!(std::ptr::eq(
            output.mod_particle.as_deref().unwrap(),
            published
        ));
    }
    #[test]
    fn particle_boundary_refuses_nonfinite_nested_input() {
        prepare();
        let good = EngineOptions::simple(ParticleType::Smoke)
            .try_into_api()
            .unwrap();
        assert!(good.try_into_engine(&()).is_ok());
        let mut color = good.clone();
        color.to_color[0] = f32::NAN;
        let mut trail = good.clone();
        trail.trail = Some(([f64::INFINITY, 0.0, 0.0], 3));
        let mut geyser = good.clone();
        geyser.geyser = Some((1, f32::NAN));
        let mut y_offset = good.clone();
        y_offset.vibration = Some((
            PositionSource::Entity {
                id: 1,
                y_offset: f32::NAN,
            },
            [0.0; 3],
            3,
        ));
        let mut destination = good;
        destination.vibration = Some((PositionSource::Block([0; 3]), [0.0, f64::NAN, 0.0], 3));
        for options in [color, trail, geyser, y_offset, destination] {
            assert_eq!(
                options.try_into_engine(&()),
                Err(ParticleValueError::NonFinite)
            );
        }
    }
    #[test]
    fn an_item_crumbs_colour_must_stay_inside_zero_to_one() {
        prepare();
        let colour = |ty, color| {
            let mut options = EngineOptions::simple(ty).try_into_api().unwrap();
            options.color = color;
            options.try_into_engine(&())
        };
        assert_eq!(
            colour(ParticleType::Item, [1.5, 0.0, 0.0, 1.0]).map(|_| ()),
            Err(ParticleValueError::ColorRange)
        );
        assert_eq!(
            colour(ParticleType::Item, [0.8, 0.5, 0.0, 1.0])
                .unwrap()
                .color,
            [0.8, 0.5, 0.0, 1.0]
        );
        // Another particle's colour is not bounded here.
        assert_eq!(
            colour(ParticleType::Smoke, [1.5, 0.0, 0.0, 1.0])
                .unwrap()
                .color,
            [1.5, 0.0, 0.0, 1.0]
        );
    }
}
