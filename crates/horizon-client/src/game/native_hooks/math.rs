use super::*;

impl Game<'_> {
    /// Mirrors jmath's branch precedence: AArch64 first, then Windows, then
    /// platform libm on every remaining target. No runtime feature selection.
    pub(in crate::game) fn math_profile(&self) -> mod_api::MathProfile {
        selected_profile()
    }
}

fn selected_profile() -> mod_api::MathProfile {
    if cfg!(target_arch = "aarch64") {
        mod_api::MathProfile::Aarch64Fma
    } else if cfg!(windows) {
        mod_api::MathProfile::CorrectlyRounded
    } else {
        mod_api::MathProfile::LibmRounding
    }
}

#[cfg(test)]
#[path = "math/tests.rs"]
mod tests;
