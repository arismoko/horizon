//! Validation of stack-valued particle sends after the API stack boundary.
use {
    sunrise_level::particle_options::ParticleOptions, sunrise_level::particle_options::ParticleType,
};
pub fn supported(options: &ParticleOptions) -> bool {
    options.ty == ParticleType::Item
        && options
            .item_stack
            .as_ref()
            .is_some_and(|stack| stack.count > 0)
}
