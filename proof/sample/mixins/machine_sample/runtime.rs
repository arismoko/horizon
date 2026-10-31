// Shared by the standalone tests and the native bundle.
pub const MACHINE: &str = "machine_sample:machine";
pub const PIPE: &str = "machine_sample:pipe";
pub const DURATION: i32 = 40;
pub const DEPOSIT: i32 = 0;

pub fn deposit_count(carried: u32, input: u32, limit: u32, compatible: bool) -> u32 {
    if compatible {
        carried.min(limit.saturating_sub(input))
    } else {
        0
    }
}

pub fn advance(progress: i32, can_process: bool) -> (i32, bool) {
    if !can_process {
        return (0, false);
    }
    let next = progress.clamp(0, DURATION - 1) + 1;
    if next == DURATION {
        (0, true)
    } else {
        (next, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deposit_respects_capacity_and_compatibility() {
        assert_eq!(deposit_count(0, 0, 64, true), 0);
        assert_eq!(deposit_count(32, 0, 64, true), 32);
        assert_eq!(deposit_count(32, 50, 64, true), 14);
        assert_eq!(deposit_count(32, 64, 64, true), 0);
        assert_eq!(deposit_count(32, 70, 64, true), 0);
        assert_eq!(deposit_count(32, 5, 64, false), 0);
    }
    #[test]
    fn completes_after_forty_ticks_and_resets_when_blocked() {
        let mut progress = 0;
        for _ in 0..39 {
            let (next, done) = advance(progress, true);
            assert!(!done);
            progress = next;
        }
        assert_eq!(advance(progress, true), (0, true));
        assert_eq!(advance(progress, false), (0, false));
    }
}
