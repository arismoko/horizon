//! The launch paths shared by the game and the launcher's mod preparation.
use std::{
    ffi::OsStr,
    fmt,
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub struct ModSelectionContext {
    pub game_dir: PathBuf,
    pub home: PathBuf,
    pub dev_mods: Vec<PathBuf>,
}

/// Arguments the game owns after the selection options have been consumed.
#[derive(Debug, PartialEq, Eq)]
pub enum Argument {
    Switch(String),
    Value(String, String),
    Dedicated(Vec<String>),
}

#[derive(Debug)]
pub struct LaunchArguments {
    pub context: ModSelectionContext,
    pub rest: Vec<Argument>,
}

#[derive(Debug)]
pub struct MissingValue(String);

impl fmt::Display for MissingValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "missing value after {}", self.0)
    }
}
impl std::error::Error for MissingValue {}

impl LaunchArguments {
    /// Resolve paths against the absolute starting directory, before the game changes it.
    /// Only selection options are interpreted; other values retain their spelling and order.
    pub fn parse(
        mut args: impl Iterator<Item = String>,
        cwd: &Path,
        default_home: &Path,
        env_home: Option<&OsStr>,
    ) -> Result<Self, MissingValue> {
        let mut context = ModSelectionContext {
            game_dir: cwd.to_path_buf(),
            home: cwd.join(
                env_home
                    .filter(|v| !v.is_empty())
                    .unwrap_or(default_home.as_os_str()),
            ),
            dev_mods: Vec::new(),
        };
        let mut rest = Vec::new();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--dedicated" => {
                    rest.push(Argument::Dedicated(args.collect()));
                    break;
                }
                "--help" | "-h" | "--launch-api" => {
                    rest.push(Argument::Switch(arg));
                    break;
                }
                "--fullscreen" | "--session-stdin" | "--creative" | "--test-terrain"
                | "--no-pause" | "--offscreen" | "--headless" | "--control" | "--server-thread"
                | "--in-frame" | "--no-server-thread" | "--pregen" => {
                    rest.push(Argument::Switch(arg));
                }
                _ => {
                    let value = args.next().ok_or_else(|| MissingValue(arg.clone()))?;
                    match arg.as_str() {
                        "--game-dir" | "--gameDir" => context.game_dir = cwd.join(value),
                        "--home" => context.home = cwd.join(value),
                        "--dev-mod" => context.dev_mods.push(cwd.join(value)),
                        _ => rest.push(Argument::Value(arg, value)),
                    }
                }
            }
        }
        Ok(Self { context, rest })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str], home: Option<&str>) -> LaunchArguments {
        LaunchArguments::parse(
            args.iter().map(|s| s.to_string()),
            Path::new("/start"),
            Path::new("/default"),
            home.map(OsStr::new),
        )
        .unwrap()
    }

    #[test]
    fn paths_follow_overrides_before_chdir_and_the_dedicated_boundary() {
        let parsed = parse(
            &[
                "--game-dir",
                "first",
                "--gameDir",
                "game",
                "--home",
                "cache",
                "--dev-mod",
                "one.hmod",
                "--dev-mod",
                "/two.hmod",
                "--dedicated",
                "--home",
                "server-only",
            ],
            Some("/environment"),
        );
        assert_eq!(parsed.context.game_dir, Path::new("/start/game"));
        assert_eq!(parsed.context.home, Path::new("/start/cache"));
        assert_eq!(
            parsed.context.dev_mods,
            [PathBuf::from("/start/one.hmod"), PathBuf::from("/two.hmod")]
        );
        assert_eq!(
            parsed.rest,
            [Argument::Dedicated(vec![
                "--home".into(),
                "server-only".into()
            ])]
        );
        assert_eq!(
            parse(&[], Some("relative")).context.home,
            Path::new("/start/relative")
        );
        assert_eq!(parse(&[], None).context.home, Path::new("/default"));
    }

    #[test]
    fn values_that_look_like_options_are_not_selection_options() {
        let parsed = parse(
            &[
                "--resources",
                "--home",
                "--creative",
                "--game-dir",
                "actual",
                "--name",
                "--dev-mod",
            ],
            Some("/environment"),
        );
        assert_eq!(parsed.context.game_dir, Path::new("/start/actual"));
        assert_eq!(parsed.context.home, Path::new("/environment"));
        assert!(parsed.context.dev_mods.is_empty());
        assert_eq!(
            parsed.rest,
            [
                Argument::Value("--resources".into(), "--home".into()),
                Argument::Switch("--creative".into()),
                Argument::Value("--name".into(), "--dev-mod".into())
            ]
        );
        let error = LaunchArguments::parse(
            ["--dev-mod".to_string()].into_iter(),
            Path::new("/start"),
            Path::new("/default"),
            None,
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "missing value after --dev-mod");
    }
}
