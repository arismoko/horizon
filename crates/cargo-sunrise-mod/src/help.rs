//! What `cargo sunrise-mod --help`, `help all`, `help <command>` and `<command> --help` print: one
//! table, every command once.

struct Command {
    name: &'static str,
    /// One of the commands a mod author uses every day: listed by plain `--help`.
    everyday: bool,
    usage: &'static str,
    summary: &'static str,
}

const COMMANDS: &[Command] = &[
    Command {
        name: "new",
        everyday: true,
        usage: "new <path> [--name \"Name\"] [--template content|native [--sunrise <build>]]",
        summary: "Makes a mod project in <path> with a page template and a release workflow; a native one is pinned to the current Nightly's SDK unless --sunrise names another build.",
    },
    Command {
        name: "build",
        everyday: true,
        usage: "build [--update-lock] [--with <path>]…",
        summary: "Compiles a native mod with its required libraries and any optional libraries supplied with --with, then packs only the mod into target/sunrise-mod/<id>.smod, and its page.toml and screenshots, if it has them, into target/sunrise-mod/<id>.page.tar. Use --with for local folders or .smod files; the Sunrise mod registry supplies other required libraries. --update-lock resolves its Cargo.lock again.",
    },
    Command {
        name: "run",
        everyday: true,
        usage: "run [--game <sunrise-client>] [--resources <folder>] [--jars <dir>] [--with <path>]… [-- <game args>]",
        summary: "Builds the mod and starts the game with its declared libraries; repeat --with for local mod folders or .smod files. --game names the game for a content mod, --resources names prepared resources for the native mod's pinned SDK, and --jars names a folder of original Java jars by SHA-512.",
    },
    Command {
        name: "check",
        everyday: true,
        usage: "check [--release <tag>] [--uses] [--with <path>]… | check <file> [--kind <kind>] [--attestation <bundle>] [--repository <id>] [--json] | check [<file.smod>] --build <name> [--build <name>]…",
        summary: "Checks the mod in this folder, including native Rust code with the same libraries as build, or validates a release file, or reports fit for the builds named by --build. Folder checks report missing page requirements; --release requires a valid page and matching version tag, and compiles native code. A .smod must have its .sigstore.json bundle beside it unless --attestation names another path. --repository requires the attestation to name that GitHub repository ID. Repeat --with for local libraries; --uses lists what the mod uses of its pinned build. Fit mode takes no other options and exits 0 if all fit, 1 if one does not, or 2 if it cannot tell.",
    },
    Command {
        name: "pin",
        everyday: true,
        usage: "pin [<build>]",
        summary: "Repins a native mod to another game build (the current Nightly if no build is given) and reports what that breaks; a content mod pins no build.",
    },
    Command {
        name: "hooks",
        everyday: true,
        usage: "hooks <text> [--all]",
        summary: "Searches the pinned build's hooks, or with --all every item of its API, offline.",
    },
    Command {
        name: "release",
        everyday: true,
        usage: crate::author_release::USAGE,
        summary: "Checks a mod in a registered repository, commits the version bump (patch by default), and pushes the release tag and branch to origin. --check previews without changing your checkout; --pull-request commits on a new release/<tag> branch for a default branch that takes only pull requests. release setup only refreshes the shared workflow caller and removes old release files.",
    },
    Command {
        name: "collection",
        everyday: false,
        usage: crate::collection::USAGE,
        summary: "Checks the registry collection kept in this folder (collection.toml and icon.png) with the registry's rules, then pushes it from your fork of sunrise-org/sunrise-mods and prints the link that opens its pull request. Without collection.toml it writes one listing this repository's published projects from the signed registry index; without an id it gives the collection a permanent one. --check stops after the checks. The registry publishes a collection only once at least 2 members, including a default, have a release.",
    },
    Command {
        name: "internals",
        everyday: false,
        usage: "internals sync | internals add <path>… [--kind <kind>]",
        summary: "Writes the game internals a native mod names into sunrise-mod.toml, found by checking its code or taken from the pinned build's catalog.",
    },
    Command {
        name: "sdk",
        everyday: false,
        usage: "sdk diff <build> <build>",
        summary: "Lists what changed in the API surface between two builds.",
    },
    Command {
        name: "use-jar",
        everyday: false,
        usage: "use-jar <path | modrinth-url | <project> <version>>",
        summary: "Lists an exact original Java jar as a passive asset and data source from a local file, a Modrinth version URL, or a Modrinth project and version, and caches it for runs.",
    },
    Command {
        name: "self-update",
        everyday: false,
        usage: "self-update",
        summary: "Updates the standalone tool to the newest signed mod-tool release. Does not change any mod's pinned SDK.",
    },
    Command {
        name: "native-build",
        everyday: false,
        usage: "native-build < plan.json",
        summary: "Compiles native mods against an unpacked SDK from a JSON plan on standard input (reference/sdk/08-native-build.md).",
    },
    Command {
        name: "native-check",
        everyday: false,
        usage: "native-check < plan.json",
        summary: "Says how native mods stand against one build before anything compiles.",
    },
    Command {
        name: "native-discover",
        everyday: false,
        usage: "native-discover < plan.json",
        summary: "Finds the game internals a native mod names, or the pinned game's build data.",
    },
    Command {
        name: "native-lock",
        everyday: false,
        usage: "native-lock < plan.json",
        summary: "Writes a native mod's registry lock from its pinned game's build data.",
    },
    Command {
        name: "native-layout",
        everyday: false,
        usage: "native-layout < plan.json",
        summary: "Checks a local prebuilt game and lays it out as an SDK's slices.",
    },
];

/// Whether this command belongs to the everyday list and can show the update notice.
pub fn is_everyday(name: &str) -> bool {
    COMMANDS.iter().any(|c| c.name == name && c.everyday)
}

/// The short list `cargo sunrise-mod --help` prints.
pub fn short() -> String {
    let mut out = String::from("usage: cargo sunrise-mod <command> [options]\n\n");
    for command in COMMANDS.iter().filter(|c| c.everyday) {
        let first = command.summary.split(';').next().unwrap_or(command.summary);
        out += &format!("  {:<8}{}\n", command.name, first.trim_end_matches('.'));
    }
    out += "\n`cargo sunrise-mod <command> --help` says more about one command.\nmore: cargo sunrise-mod help all\n";
    out
}

/// Author-facing commands' usage, one per line: `help all`.
pub fn all() -> String {
    COMMANDS
        .iter()
        .filter(|c| !c.name.starts_with("native-"))
        .map(|c| format!("cargo sunrise-mod {}\n", c.usage))
        .collect()
}

/// One command's usage and what it does, when `name` is a command.
pub fn of(name: &str) -> Option<String> {
    let command = COMMANDS.iter().find(|c| c.name == name)?;
    Some(format!(
        "usage: cargo sunrise-mod {}\n\n{}\n",
        command.usage, command.summary
    ))
}

/// The text for a `help` command's arguments, or why it names no command.
pub fn run(args: &[String]) -> Result<String, String> {
    match args.first().map(String::as_str) {
        None => Ok(short()),
        Some("all") => Ok(all()),
        Some(name) => of(name).ok_or_else(|| unknown(name)),
    }
}

/// The error for a word that is not a command.
pub fn unknown(name: &str) -> String {
    format!("{name:?} is not a cargo sunrise-mod command; `cargo sunrise-mod --help` lists them")
}

/// Whether `args` (a command's, after its name) ask for its help: `--help` or `-h` right after the
/// command. Later it may be an option's value (`--with --help`) or meant for the game (`run -- --help`).
pub fn asked(args: &[String]) -> bool {
    args.first().is_some_and(|a| a == "--help" || a == "-h")
}
