//! The author-side release flow. GitHub runs the shared workflow after the tag is pushed.
use crate::{internals, options::Options, read_manifest};
use mod_surface::child::ChildEntry;
use semver::Version;
use serde::Deserialize;
use std::{
    ffi::OsStr,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    process::{Command, Output},
};
use sunrise_mod_package::{Manifest, manifest::MANIFEST};
use toml_edit::DocumentMut;

pub(crate) const USAGE: &str =
    "release [<version>|patch|minor|major] [--check] [--pull-request] | release setup";
const WORKFLOW: &str = "sunrise-org/sunrise-mods/.github/workflows/release.yml@release-v";
const DEFAULT_WORKFLOW: &str = "sunrise-org/sunrise-mods/.github/workflows/release.yml@release-v1";
const CALLER: &str = ".github/workflows/sunrise-release.yml";
const CALLER_MARK: &str =
    "# Written by `cargo sunrise-mod release`, which rewrites it: edits here are replaced.\n";
const NOT_REGISTERED: &str = "This repository isn't registered yet: register it at https://playsunrise.net/mods/submit, or with a Register issue on https://github.com/sunrise-org/sunrise-mods";

pub(crate) type Fetch<'a> = &'a dyn Fn(&str) -> Result<Option<String>, String>;

pub(crate) fn fetch(url: &str) -> Result<Option<String>, String> {
    let authorization = github_authorization(url, |name| std::env::var(name).ok());
    let mut headers = vec![("User-Agent", "cargo-sunrise-mod")];
    if let Some(authorization) = &authorization {
        headers.push(("Authorization", authorization));
    }
    match sunrise_proxy::get(url, &headers, None) {
        Ok(response) => response
            .into_string()
            .map(Some)
            .map_err(|e| format!("{url}: {e}")),
        Err(e) if e.status() == Some(404) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// `Bearer <token>` for a GitHub API request (`https://api.github.com/`) when `GH_TOKEN` or
/// `GITHUB_TOKEN` is set, in that order as the GitHub CLI reads them, so a batch of releases is
/// not held to the 60 unauthenticated requests an hour. No other host is sent the token, and
/// `sunrise_proxy::get` does not carry it to a redirect's destination.
fn github_authorization(url: &str, var: impl Fn(&str) -> Option<String>) -> Option<String> {
    if !url.starts_with("https://api.github.com/") {
        return None;
    }
    ["GH_TOKEN", "GITHUB_TOKEN"]
        .into_iter()
        .filter_map(var)
        .map(|token| token.trim().to_owned())
        .find(|token| !token.is_empty())
        .map(|token| format!("Bearer {token}"))
}

pub(crate) fn registry_file(fetch: Fetch<'_>, name: &str) -> Result<String, String> {
    let url = format!("{}/{name}", sunrise_addresses::MOD_REGISTRY_METADATA);
    fetch(&url)?.ok_or_else(|| format!("{url}: not found"))
}

pub(crate) fn github_name(origin: &str) -> Option<String> {
    github_name_from_origin(origin, url::Url::parse(origin).ok().as_ref())
}

fn github_name_from_origin(origin: &str, url: Option<&url::Url>) -> Option<String> {
    let path = if origin.contains("://") {
        let url = url?;
        if !matches!(url.scheme(), "https" | "ssh")
            || !url.host_str()?.eq_ignore_ascii_case("github.com")
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return None;
        }
        url.path().strip_prefix('/')?
    } else {
        let (authority, path) = origin.split_once(':')?;
        let host = authority
            .rsplit_once('@')
            .map_or(authority, |(_, host)| host);
        if !host.eq_ignore_ascii_case("github.com") {
            return None;
        }
        path
    };
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let (owner, repo) = path.split_once('/')?;
    let valid = |part: &str| {
        !matches!(part, "" | "." | "..")
            && part
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
    };
    (valid(owner) && valid(repo)).then(|| path.to_owned())
}

/// GitHub's page for opening a pull request from `head` into `base` in `repository`.
fn pull_request_url(repository: &str, base: &str, head: &str) -> String {
    format!("https://github.com/{repository}/compare/{base}...{head}?expand=1")
}

/// Resolve the stable GitHub ID of a parsed `owner/name`, since a name can be reused after a rename.
pub(crate) fn github_repository_id(fetch: Fetch<'_>, name: &str) -> Result<u64, String> {
    #[derive(Deserialize)]
    struct GitHubRepository {
        id: u64,
    }
    let url = format!("https://api.github.com/repos/{name}");
    let text = fetch(&url)?.ok_or(NOT_REGISTERED)?;
    let GitHubRepository { id } = serde_json::from_str(&text).map_err(|e| format!("{url}: {e}"))?;
    Ok(id)
}

fn registered_repository_name(fetch: Fetch<'_>, origin: &str) -> Result<String, String> {
    let url = url::Url::parse(origin).ok();
    // Git would pass a token embedded in the URL on to every command line that names it.
    if url.as_ref().is_some_and(|url| {
        matches!(url.scheme(), "http" | "https")
            && (!url.username().is_empty() || url.password().is_some())
    }) {
        return Err("origin's push URL carries credentials; remove them and use a Git credential helper or SSH".into());
    }
    let name = github_name_from_origin(origin, url.as_ref()).ok_or(NOT_REGISTERED)?;
    let id = github_repository_id(fetch, &name)?;
    let url = format!(
        "{}/repositories/{id}.toml",
        sunrise_addresses::MOD_REGISTRY_METADATA
    );
    fetch(&url)?.ok_or(NOT_REGISTERED)?;
    Ok(name)
}

fn workflow(fetch: Fetch<'_>) -> Result<String, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Workflow {
        uses: String,
    }
    let Workflow { uses } = toml::from_str(&registry_file(fetch, "release-workflow.toml")?)
        .map_err(|e| format!("registry/release-workflow.toml: {e}"))?;
    if !uses
        .strip_prefix(WORKFLOW)
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(format!(
            "registry/release-workflow.toml: expected uses = \"{WORKFLOW}<number>\""
        ));
    }
    Ok(uses)
}

fn caller(prefix: &str, path: &str, uses: &str) -> String {
    // Ordinary paths stay plain YAML. Quote strings that YAML might coerce, strip or interpret.
    let path = if path == "."
        || (path.starts_with(|c: char| c.is_ascii_alphabetic())
            && path
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_.-/".contains(&c))
            && !matches!(
                path.to_ascii_lowercase().as_str(),
                "true" | "false" | "null" | "yes" | "no" | "on" | "off" | "y" | "n"
            )) {
        path.into()
    } else {
        serde_json::json!(path).to_string()
    };
    format!(
        "{CALLER_MARK}\
name: Sunrise release
on:
  push:
    tags: ['{prefix}v*']
permissions: {{}}
jobs:
  release:
    permissions:
      contents: write
      id-token: write
      attestations: write
    uses: {uses}
    with:
      path: {path}
"
    )
}

pub(crate) fn scaffold(root: &Path) -> Result<(), String> {
    Change::write(CALLER, caller("", ".", DEFAULT_WORKFLOW)).apply(root)
}

/// A package and its caller are both addressed relative to the Git root.
struct Project {
    root: PathBuf,
    package: PathBuf,
    path: String,
    prefix: String,
    caller: PathBuf,
    manifest: Manifest,
}

impl Project {
    fn read(package: &Path, git: &Git) -> Result<Self, String> {
        let root = git.text(package, ["rev-parse", "--show-toplevel"])?;
        let canonical =
            |path: &Path| dunce::canonicalize(path).map_err(|e| format!("{}: {e}", path.display()));
        let root = canonical(Path::new(&root))?;
        let package = canonical(package)?;
        let relative = package
            .strip_prefix(&root)
            .map_err(|_| "the package must be in the Git working tree")?;
        let mut pieces = Vec::new();
        for part in relative.iter() {
            pieces.push(part.to_str().ok_or("the package path must be UTF-8")?);
        }
        let manifest = read_manifest(&package)?;
        let (path, prefix, caller) = if pieces.is_empty() {
            (".".into(), String::new(), CALLER.into())
        } else {
            let path = pieces.join("/");
            if path.contains("${{") || path.contains('\\') {
                return Err(
                    "the package path cannot be passed literally to the release workflow".into(),
                );
            }
            let id = &manifest.id;
            (
                path,
                format!("{id}/"),
                format!(".github/workflows/sunrise-release-{id}.yml").into(),
            )
        };
        let package = relative.to_path_buf();
        Ok(Self {
            root,
            package,
            path,
            prefix,
            caller,
            manifest,
        })
    }

    /// The caller to write and the old copied release files to remove. A file at the caller's path
    /// that this tool did not write, tracked or not, is refused rather than replaced; only committed
    /// old files are removed, so each removal can be restored from Git.
    fn workflow_changes(&self, git: &Git, uses: &str) -> Result<Vec<Change>, String> {
        let root = &self.root;
        for folder in [".github", ".github/workflows"] {
            internals::check_output_dir(&root.join(folder))?;
        }
        let ours = |text: &str| text.starts_with(CALLER_MARK);
        if let Some(meta) = metadata(&root.join(&self.caller))?
            && (!meta.is_file()
                || !git.tracked(root, &self.caller)?
                || !fs::read_to_string(root.join(&self.caller)).is_ok_and(|text| ours(&text)))
        {
            return Err(format!(
                "{} exists and was not written by cargo sunrise-mod; move it aside before releasing",
                self.caller.display()
            ));
        }
        let mut changes = vec![Change::write(
            &self.caller,
            caller(&self.prefix, &self.path, uses),
        )];
        // The old copied files at the repository's root served the root package: a package in a
        // subfolder gets its own caller and leaves them to the package they serve.
        let at_root = self.package.as_os_str().is_empty();
        // These names aren't reserved. Only remove files with the old templates' signatures.
        let workflows = [
            ("release.yml", "Release native mod"),
            ("rebuild.yml", "Rebuild reviewed native mod source"),
            (
                "sunrise-release-jobs.yml",
                "Build and publish a planned mod release",
            ),
        ];
        for (file, name) in workflows.into_iter().filter(|_| at_root) {
            let path = Path::new(".github/workflows").join(file);
            let title = format!("name: {name}");
            if legacy_file(
                git,
                root,
                &path,
                &[&title, "./.github/actions/sunrise-release-tool"],
            )? {
                changes.push(Change::remove(path, false));
            }
        }
        let actions = Path::new(".github/actions");
        let action = actions.join("sunrise-release-tool");
        let script = action.join("action.yml");
        if at_root
            && metadata(&root.join(actions))?.is_some_and(|meta| meta.is_dir())
            && metadata(&root.join(action))?.is_some_and(|meta| meta.is_dir())
            && legacy_file(
                git,
                root,
                &script,
                &[
                    "Install the verified Sunrise release tool",
                    "SUNRISE_RELEASE_SCRIPTS",
                ],
            )?
        {
            changes.push(Change::remove(script, true));
        }
        for (file, signature) in [
            (
                "sunrise_release.py",
                "Build, sign and stage a native mod release",
            ),
            (
                "sunrise_release_github.py",
                "from sunrise_release import Batch",
            ),
        ] {
            let path = self.package.join(file);
            if legacy_file(git, root, &path, &[signature])? {
                changes.push(Change::remove(path, false));
            }
        }
        Ok(changes)
    }

    fn version_changes(&self, git: &Git, version: &Version) -> Result<Vec<Change>, String> {
        let version = version.to_string();
        let path = self.package.join(MANIFEST);
        let mut mod_doc = read_tracked_toml(git, &self.root, &path)?;
        internals::set_string(
            mod_doc["version"]
                .as_value_mut()
                .expect("parsed manifest version"),
            &version,
        );
        let mut changes = vec![Change::write(path, mod_doc.to_string())];
        if self.manifest.native.is_none() {
            return Ok(changes);
        }
        let cargo_path = self.package.join("Cargo.toml");
        let lock_path = self.package.join("Cargo.lock");
        let mut cargo = read_tracked_toml(git, &self.root, &cargo_path)?;
        let mut lock = read_tracked_toml(git, &self.root, &lock_path)?;
        let package = cargo
            .get_mut("package")
            .and_then(|p| p.as_table_like_mut())
            .ok_or("Cargo.toml: no [package] table")?;
        let name = package
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or("Cargo.toml: no package name")?
            .to_owned();
        let value = package
            .get_mut("version")
            .and_then(|v| v.as_value_mut())
            .ok_or("Cargo.toml: [package] must specify a version")?;
        let old = value
            .as_str()
            .ok_or("Cargo.toml: [package] must specify a version string")?;
        let packages = lock
            .get_mut("package")
            .and_then(|p| p.as_array_of_tables_mut())
            .ok_or("Cargo.lock: no package entries")?;
        // A root package entry has no source. Update it without running Cargo's resolver or
        // rewriting the checksums/versions of the SDK and crates.io dependencies.
        {
            let mut entries = packages.iter_mut().filter(|p| {
                p.get("name").and_then(|v| v.as_str()) == Some(&name)
                    && p.get("version").and_then(|v| v.as_str()) == Some(old)
                    && !p.contains_key("source")
            });
            let own = entries.next().ok_or_else(|| format!("Cargo.lock: no local entry for {name} {old}; refresh Cargo.lock before releasing"))?;
            if entries.next().is_some() {
                return Err(format!(
                    "Cargo.lock: more than one local entry for {name} {old}"
                ));
            }
            internals::set_string(
                own["version"]
                    .as_value_mut()
                    .expect("matched version string"),
                &version,
            );
        }
        // Cargo qualifies a dependency by version when several locked packages share a name.
        let old_id = format!("{name} {old}");
        let new_id = format!("{name} {version}");
        for p in packages.iter_mut() {
            if let Some(deps) = p.get_mut("dependencies").and_then(|v| v.as_array_mut()) {
                for dep in deps.iter_mut().filter(|v| v.as_str() == Some(&old_id)) {
                    internals::set_string(dep, &new_id);
                }
            }
        }
        internals::set_string(value, &version);
        changes.push(Change::write(cargo_path, cargo.to_string()));
        changes.push(Change::write(lock_path, lock.to_string()));
        Ok(changes)
    }
}

fn metadata(path: &Path) -> Result<Option<fs::Metadata>, String> {
    match fs::symlink_metadata(path) {
        Ok(meta) => Ok(Some(meta)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// A committed regular file carrying every one of an old template's `signatures`. Untracked and
/// ignored files are never matched, so nothing that Git could not restore is removed.
fn legacy_file(
    git: &Git,
    root: &Path,
    relative: &Path,
    signatures: &[&str],
) -> Result<bool, String> {
    let path = root.join(relative);
    if !metadata(&path)?.is_some_and(|m| m.is_file()) || !git.tracked(root, relative)? {
        return Ok(false);
    }
    let bytes = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(std::str::from_utf8(&bytes).is_ok_and(|text| signatures.iter().all(|&s| text.contains(s))))
}

fn read_tracked_toml(git: &Git, root: &Path, relative: &Path) -> Result<DocumentMut, String> {
    let path = root.join(relative);
    if !metadata(&path)?.is_some_and(|m| m.is_file()) || !git.tracked(root, relative)? {
        return Err(format!(
            "{} must be a committed regular file before releasing",
            relative.display()
        ));
    }
    fs::read_to_string(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .parse()
        .map_err(|e| format!("{}: {e}", path.display()))
}

struct Change {
    path: PathBuf,
    kind: Kind,
}

enum Kind {
    Write(String),
    Remove { prune_parent: bool },
}

impl Change {
    fn write(path: impl Into<PathBuf>, text: String) -> Self {
        Self {
            path: path.into(),
            kind: Kind::Write(text),
        }
    }

    fn remove(path: PathBuf, prune_parent: bool) -> Self {
        Self {
            path,
            kind: Kind::Remove { prune_parent },
        }
    }

    fn apply(&self, root: &Path) -> Result<(), String> {
        let path = root.join(&self.path);
        let parent = path.parent().expect("path in checkout");
        match &self.kind {
            Kind::Write(text) => {
                fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
                internals::atomic_write(&path, text)
            }
            Kind::Remove { prune_parent } => {
                if let Err(e) = fs::remove_file(&path)
                    && e.kind() != ErrorKind::NotFound
                {
                    return Err(format!("{}: {e}", path.display()));
                }
                if *prune_parent
                    && let Err(e) = fs::remove_dir(parent)
                    && !matches!(e.kind(), ErrorKind::NotFound | ErrorKind::DirectoryNotEmpty)
                {
                    return Err(format!("{}: {e}", parent.display()));
                }
                Ok(())
            }
        }
    }

    fn describe(&self) {
        let verb = match self.kind {
            Kind::Write(_) => "Write",
            Kind::Remove { .. } => "Remove",
        };
        println!("  {verb} {}", self.path.display());
    }
}

#[derive(Default)]
pub(crate) struct Git {
    #[cfg(test)]
    pub(crate) env: Vec<(&'static str, std::ffi::OsString)>,
}

impl Git {
    fn command(&self, root: &Path) -> Command {
        let mut cmd = Command::new("git");
        cmd.current_dir(root);
        #[cfg(test)]
        cmd.envs(self.env.iter().map(|(key, value)| (*key, value)));
        cmd
    }

    pub(crate) fn text(
        &self,
        root: &Path,
        args: impl IntoIterator<Item: AsRef<OsStr>>,
    ) -> Result<String, String> {
        let mut cmd = self.command(root);
        cmd.args(args);
        let Output {
            status,
            stdout,
            stderr,
        } = cmd.output().map_err(|e| format!("git: {e}"))?;
        if !status.success() {
            let operation = cmd.get_args().next().unwrap_or_default().to_string_lossy();
            return Err(format!(
                "git {operation}: {}",
                String::from_utf8_lossy(&stderr).trim_end()
            ));
        }
        let stdout = String::from_utf8(stdout).map_err(|e| format!("git: {e}"))?;
        Ok(stdout
            .strip_suffix("\r\n")
            .or_else(|| stdout.strip_suffix('\n'))
            .unwrap_or(&stdout)
            .to_owned())
    }

    fn tracked(&self, root: &Path, path: &Path) -> Result<bool, String> {
        Ok(!self
            .text(
                root,
                ["--literal-pathspecs", "ls-files", "--"]
                    .map(OsStr::new)
                    .into_iter()
                    .chain(std::iter::once(path.as_os_str())),
            )?
            .is_empty())
    }

    fn clean(&self, root: &Path) -> Result<(), String> {
        if !self
            .text(
                root,
                [
                    "--no-optional-locks",
                    "status",
                    "--porcelain",
                    "--untracked-files=all",
                    "--ignore-submodules=none",
                ],
            )?
            .is_empty()
        {
            return Err(
                "the Git work tree has uncommitted changes; commit or stash them before releasing"
                    .into(),
            );
        }
        // A pending merge, cherry-pick, revert, rebase or bisect can leave a clean status; the
        // release commit would complete it.
        for state in [
            "MERGE_HEAD",
            "CHERRY_PICK_HEAD",
            "REVERT_HEAD",
            "rebase-merge",
            "rebase-apply",
            "sequencer",
            "BISECT_LOG",
        ] {
            let path = self.text(
                root,
                ["rev-parse", "--path-format=absolute", "--git-path", state],
            )?;
            if metadata(Path::new(&path))?.is_some() {
                return Err(format!(
                    "a Git operation is in progress ({state}); finish or abort it before releasing"
                ));
            }
        }
        Ok(())
    }
}

fn next_version(current: &Version, requested: &str) -> Result<Version, String> {
    let bump = |part: u64| {
        part.checked_add(1)
            .ok_or("the version component cannot be incremented")
    };
    let next = match requested {
        "major" => Version::new(bump(current.major)?, 0, 0),
        "minor" => Version::new(current.major, bump(current.minor)?, 0),
        "patch" => Version::new(current.major, current.minor, bump(current.patch)?),
        text => Version::parse(text).map_err(|e| format!("release version {text:?}: {e}"))?,
    };
    if !next.cmp_precedence(current).is_gt() {
        return Err(format!(
            "release version {next} must be higher than {current}"
        ));
    }
    Ok(next)
}

pub(crate) fn run(entry: ChildEntry, root: &Path, args: &[String]) -> Result<(), String> {
    release(root, args, &Git::default(), &fetch, |root, tag| {
        crate::sdk_cmds::check_mod(entry, root, false, &[], Some(tag))
    })
}

fn release(
    package: &Path,
    args: &[String],
    git: &Git,
    fetch: Fetch<'_>,
    check: impl FnOnce(&Path, &str) -> Result<(), String>,
) -> Result<(), String> {
    if let Some((command, rest)) = args.split_first()
        && command == "setup"
    {
        Options::new("release setup", rest, &[]).finish()?;
        let project = Project::read(package, git)?;
        // A clean tree means every file setup replaces or removes is committed as it is on disk.
        git.clean(&project.root)?;
        for change in project.workflow_changes(git, &workflow(fetch)?)? {
            change.apply(&project.root)?;
            change.describe();
        }
        println!(
            "Review these changes and commit them; `git restore <path>` brings back a file you want to keep."
        );
        return Ok(());
    }
    let mut options = Options::new("release", args, &[]);
    let preview = options.switch("--check");
    let pull_request = options.switch("--pull-request");
    let requested = options.at_most_one()?.unwrap_or("patch");
    options.finish()?;
    let project = Project::read(package, git)?;
    let root = &project.root;
    git.clean(root)?;
    let branch = git
        .text(root, ["symbolic-ref", "--quiet", "HEAD"])
        .map_err(|_| "check out a branch before releasing")?;
    let head = git.text(root, ["rev-parse", "HEAD"])?;
    // Registration and the existing-tag and branch preflight read origin's one push URL, with
    // Git's insteadOf and pushInsteadOf rewriting already applied. The pushes go through the named
    // remote, which Git resolves to that same URL, and each of the two refs is checked again on the
    // repository it actually reaches.
    let origin = git.text(root, ["remote", "get-url", "--push", "--all", "origin"])?;
    if origin.lines().count() != 1 {
        return Err("origin pushes to more than one URL; release needs exactly one".into());
    }
    let repository = registered_repository_name(fetch, &origin)?;
    let version = next_version(&project.manifest.version, requested)?;
    let tag = format!("{}v{version}", project.prefix);
    let tag_ref = format!("refs/tags/{tag}");
    if !git.text(root, ["tag", "--list", &tag])?.is_empty() {
        return Err(format!("tag {tag} already exists locally"));
    }
    // With --pull-request the release commit goes on a new branch, for a default branch that takes
    // only pull requests. The tag need not be on the default branch for the workflow to run.
    let release_branch = pull_request.then(|| format!("release/{tag}"));
    let release_ref = release_branch
        .as_ref()
        .map(|name| format!("refs/heads/{name}"));
    if let Some(reference) = &release_ref
        && !git.text(root, ["for-each-ref", reference])?.is_empty()
    {
        return Err(format!("branch {reference} already exists locally"));
    }
    let mut query = vec!["ls-remote", "--refs", "--", &origin, &branch, &tag_ref];
    query.extend(release_ref.as_deref());
    let remote = git.text(root, query)?;
    let find_ref = |name: &str| {
        remote
            .lines()
            .filter_map(|line| line.split_once('\t'))
            .find_map(|(oid, r)| (r == name).then_some(oid))
    };
    if find_ref(&tag_ref).is_some() {
        return Err(format!("tag {tag} already exists on origin"));
    }
    if let Some(reference) = release_ref.as_deref().filter(|r| find_ref(r).is_some()) {
        return Err(format!("branch {reference} already exists on origin"));
    }
    let base = branch.strip_prefix("refs/heads/").unwrap_or(&branch);
    let target = release_ref.as_deref().unwrap_or(&branch);
    let upstream = git.text(
        root,
        [
            "for-each-ref",
            "--format=%(upstream:remotename)%00%(upstream:remoteref)",
            &branch,
        ],
    )?;
    let upstream_refs = match upstream.split_once('\0') {
        Some((name, reference))
            if !name.is_empty()
                && !reference.is_empty()
                && (name != "origin"
                    || reference != branch
                    || git.text(root, ["remote", "get-url", "origin"])? != origin) =>
        {
            git.text(root, ["ls-remote", "--refs", "--", name, reference])?
        }
        _ => String::new(),
    };
    let upstream_head = upstream_refs.split_once('\t').map(|(oid, _)| oid);
    for oid in [find_ref(&branch), upstream_head].into_iter().flatten() {
        let status = git
            .command(root)
            .args(["merge-base", "--is-ancestor", oid, &head])
            .output()
            .map_err(|e| format!("git merge-base: {e}"))?
            .status;
        if !status.success() {
            return Err("the branch is not up to date with its upstream or origin; fetch and integrate the remote branch before releasing".into());
        }
    }

    // The tree is clean, so each old copied file removed here stays in the previous commit.
    let mut changes = project.workflow_changes(git, &workflow(fetch)?)?;
    changes.extend(project.version_changes(git, &version)?);

    // The same check as CI sees the proposed version and tag, not the previous version.
    // Clone to a separate Git repository: the check can fetch an SDK and build native code,
    // without writing version files, build output or worktree metadata in the author's repo.
    let scratch = tempfile::tempdir().map_err(|e| format!("release check: {e}"))?;
    let snapshot = scratch.path().join("checkout");
    git.text(
        root,
        [
            OsStr::new("clone"),
            OsStr::new("--shared"),
            OsStr::new("--no-checkout"),
            OsStr::new("--"),
            root.as_os_str(),
            snapshot.as_os_str(),
        ],
    )?;
    git.text(&snapshot, ["checkout", "--detach", &head])?;
    for change in &changes {
        change.apply(&snapshot)?;
    }
    let stage = |dir: &Path| {
        git.text(
            dir,
            ["--literal-pathspecs", "add", "--"]
                .map(OsStr::new)
                .into_iter()
                .chain(changes.iter().map(|c| c.path.as_os_str())),
        )
    };
    // The tree the check is given: the release commit must have exactly this tree.
    stage(&snapshot)?;
    let checked = git.text(&snapshot, ["write-tree"])?;
    check(&snapshot.join(&project.package), &tag)?;
    if preview {
        println!("Would release {version}:");
        for change in changes {
            change.describe();
        }
        println!(
            "  Commit as Release {version} (signed off), tag {tag}, and push {target} and {tag_ref} to origin"
        );
        if let Some(name) = &release_branch {
            println!("  The commit goes on a new branch {name}, for a pull request into {base}");
        }
        return Ok(());
    }

    git.clean(root)?;
    if git.text(root, ["rev-parse", "HEAD"])? != head
        || git.text(root, ["symbolic-ref", "--quiet", "HEAD"])? != branch
    {
        return Err(
            "the checked-out branch changed during the release check; run release again".into(),
        );
    }
    if let Some(name) = &release_branch {
        git.text(root, ["switch", "--quiet", "--create", name, &head])?;
    }
    // A release only writes: the caller and the version files. Stage exactly those.
    for change in &changes {
        change.apply(root)?;
    }
    stage(root)?;
    // Signed off as the committer, for repositories that check the Developer Certificate of Origin.
    git.text(
        root,
        ["commit", "--signoff", "-m", &format!("Release {version}")],
    )?;
    // A commit hook could change what is committed after the check: tag and push only a commit
    // whose tree is the checked one, by its ID.
    let commit = git.text(root, ["rev-parse", "HEAD"])?;
    if git.text(root, ["rev-parse", &format!("{commit}^{{tree}}")])? != checked {
        return Err(format!(
            "the release commit is not the tree that was checked (a Git hook changed it?); the commit remains local and {tag} was not made"
        ));
    }
    git.text(root, ["tag", &tag, &commit])?;
    // GitHub starts no workflow for a tag pushed in the same push that first adds its workflow file,
    // so the branch goes first and the tag in a second push: a rejected branch publishes no tag.
    // Explicit destinations, so no remote.origin.push mapping sends them elsewhere.
    let branch_spec = format!("{commit}:{target}");
    let tag_spec = format!("{tag_ref}:{tag_ref}");
    git.text(root, ["push", "--no-follow-tags", "origin", &branch_spec])
        .map_err(|e| {
            let protected = match &release_branch {
                Some(_) => String::new(),
                None => format!("\nIf origin accepts {base} only through pull requests, undo with `git tag -d {tag}` and `git reset --keep HEAD~1`, then run release {version} --pull-request."),
            };
            format!("{e}\nThe release commit and {tag} remain local. Resolve the push error, then push both refs to origin.{protected}")
        })?;
    git.text(root, ["push", "--no-follow-tags", "origin", &tag_spec]).map_err(|e| {
        format!(
            "{e}\nThe release commit is on origin and only {tag} remains local. Resolve the push error, then push it with `git push origin {tag_ref}`."
        )
    })?;
    println!(
        "Released {version}: pushed {tag} to origin. GitHub Actions will build and attest the release."
    );
    if let Some(name) = &release_branch {
        let page = pull_request_url(&repository, base, name);
        println!(
            "Open a pull request from {name} into {base} and merge it, so {base} carries the version before the next release. {page}"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests;
