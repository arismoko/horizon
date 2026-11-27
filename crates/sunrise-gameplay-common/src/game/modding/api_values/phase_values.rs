//! Phase vocabulary enters through the same game-local conversion boundary.
use super::TryIntoApi;
use std::convert::Infallible;
use sunrise_world::native_load_context::{DataLoadOrigin, LoopSide};

impl TryIntoApi for DataLoadOrigin {
    type Api = mod_api::DataLoadOrigin;
    type Error = Infallible;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(sunrise_level::api_context::load_origin(*self))
    }
}
impl TryIntoApi for LoopSide {
    type Api = mod_api::LoopSide;
    type Error = Infallible;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(sunrise_level::api_context::loop_side(*self))
    }
}
