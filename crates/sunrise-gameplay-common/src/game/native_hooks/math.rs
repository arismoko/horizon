use super::*;

impl crate::game::CommonGame<'_> {
    /// The profile jmath's operations dispatch on (`jmath::PLATFORM`), in the
    /// API's own type.
    pub fn math_profile(&self) -> mod_api::MathProfile {
        selected_profile()
    }
}

/// The profile jmath's operations dispatch on before a `Game` instance exists.
pub fn startup_math_profile() -> mod_api::MathProfile {
    selected_profile()
}

pub fn selected_profile() -> mod_api::MathProfile {
    use sunrise_core::jmath::Platform;
    match sunrise_core::jmath::PLATFORM {
        Platform::Aarch64 => mod_api::MathProfile::Aarch64Fma,
        // jmath runs HotSpot's x86-64 stubs on every x86-64 target, Windows
        // included (see its sin, cos, tan and exp).
        Platform::Windows | Platform::Libm if cfg!(target_arch = "x86_64") => {
            mod_api::MathProfile::HotSpotX86_64
        }
        Platform::Windows => mod_api::MathProfile::CorrectlyRounded,
        Platform::Libm => mod_api::MathProfile::LibmRounding,
    }
}
