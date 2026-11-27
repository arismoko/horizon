//! `cargo sunrise-mod collection [--check]`: a registry collection from the folder an author keeps
//! it in (`collection.toml` and `icon.png`), checked with the registry's own rules
//! ([`check_collection`]) and proposed to sunrise-org/sunrise-mods from the author's fork, as
//! `release --pull-request` proposes its branch: pushed with plain `git`, then GitHub's page for
//! opening the pull request. The registry checks again who opens it.
use crate::{
    author_release::{self, Fetch, Git},
    options::Options,
    registry_source::Registry,
};
use serde::Deserialize;
use std::{collections::BTreeMap, fs, io::Read, path::Path};
use sunrise_addresses::{MOD_REGISTRY_METADATA, MOD_REGISTRY_REPOSITORY};
use sunrise_mod_package::{
    icon::MAX_ICON,
    index::v2::{
        Index,
        collection::{Collection, ICON, Registered, TOML, check_collection, id},
    },
};

pub(crate) const USAGE: &str = "collection [--check]";

/// `registry/projects/<id>/project.toml`: the fields a collection's checks read.
#[derive(Deserialize)]
struct ProjectRecord {
    owners: Vec<String>,
    repository: u64,
}

/// Only the publisher's `owner/name` is needed from the repository row. The registry owns the
/// rest of its schema, including legacy package lists and registrant metadata.
#[derive(Deserialize)]
struct Repository {
    name: String,
}

#[derive(Deserialize)]
struct Fork {
    fork: bool,
    parent: Option<Parent>,
}

#[derive(Deserialize)]
struct Parent {
    full_name: String,
}

pub(crate) fn run(dir: &Path, args: &[String]) -> Result<(), String> {
    let mut options = Options::new("collection", args, &[]);
    let preview = options.switch("--check");
    options.finish()?;
    let git = Git::default();
    let fetch: Fetch<'_> = &author_release::fetch;
    let file = dir.join(TOML);
    let text = match fs::read_to_string(&file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let origin = origin(dir, &git)?;
            let projects = published_from(fetch, &origin, || {
                Ok(Registry::read(&crate::dev::cache_dir()?)?.index)
            })?;
            fs::write(&file, scaffold(&projects))
                .map_err(|e| format!("{}: {e}", file.display()))?;
            println!(
                "Wrote {TOML} with {} published projects from this repository. Fill in name and summary, add a square PNG {ICON} of at most 512×512 pixels and 256 KiB, then run `cargo sunrise-mod collection` again.",
                projects.len()
            );
            return Ok(());
        }
        Err(e) => return Err(format!("{}: {e}", file.display())),
    };
    let text = match with_id(&text, &|| fresh_id(fetch))? {
        Some((text, id)) => {
            fs::write(&file, &text).map_err(|e| format!("{}: {e}", file.display()))?;
            println!("Gave the collection the ID {id}. Commit {TOML}: the ID is permanent.");
            text
        }
        None => text,
    };
    let collection: Collection =
        toml::from_str(&text).map_err(|e| format!("{}: {e}", file.display()))?;
    let icon = read_icon(&dir.join(ICON))?;
    let owners = check(fetch, &collection, icon.as_deref())?;
    let exists = fetch(&format!(
        "{MOD_REGISTRY_METADATA}/collections/{}/{TOML}",
        collection.id
    ))?
    .is_some();
    let folder = format!("registry/collections/{}", collection.id);
    let opener = format!(
        "The pull request must be opened by one of: {}.",
        owners.join(", ")
    );
    if preview {
        println!("Would propose {folder}/{TOML} and {folder}/{ICON}. {opener}");
        return Ok(());
    }
    let origin = origin(dir, &git)?;
    let name = author_release::github_name(&origin).ok_or(
        "origin is not a GitHub repository: the collection's pull request comes from a fork on GitHub",
    )?;
    let owner = name
        .split_once('/')
        .map_or(name.as_str(), |(owner, _)| owner);
    let upstream = MOD_REGISTRY_REPOSITORY
        .strip_prefix("https://github.com/")
        .expect("a GitHub repository");
    let registry = upstream.rsplit_once('/').map_or(upstream, |(_, repo)| repo);
    let by_hand = format!(
        "add {folder}/{TOML} and {folder}/{ICON} from this folder in a pull request on {MOD_REGISTRY_REPOSITORY}"
    );
    let api = format!("https://api.github.com/repos/{owner}/{registry}");
    let forked = match fetch(&api)? {
        Some(text) => {
            let fork: Fork = serde_json::from_str(&text).map_err(|e| format!("{api}: {e}"))?;
            fork.fork
                && fork
                    .parent
                    .is_some_and(|parent| parent.full_name.eq_ignore_ascii_case(upstream))
        }
        None => false,
    };
    if !forked {
        return Err(format!(
            "{owner} has no fork of {upstream}: fork it at {MOD_REGISTRY_REPOSITORY}/fork and run again. \
             If {owner} is an organisation, the pull request comes from the own fork of one of: {}; \
             {by_hand}.",
            owners.join(", ")
        ));
    }
    let fork = origin.replacen(&name, &format!("{owner}/{registry}"), 1);
    let branch = format!("collection-{}", collection.id);
    let verb = if exists { "Update" } else { "Add" };
    let message = format!("{verb} the {} collection", collection.name.trim());
    let files = [
        (TOML, text.into_bytes()),
        (ICON, icon.expect("checked above")),
    ];
    propose(
        &git,
        &format!("{MOD_REGISTRY_REPOSITORY}.git"),
        &fork,
        &branch,
        &folder,
        &files,
        &message,
    )
    .map_err(|e| {
        format!("{e}\nIf your fork is behind, sync it with {upstream} and run again, or {by_hand}.")
    })?;
    println!(
        "Pushed {branch} to {owner}/{registry}. Open the pull request: {MOD_REGISTRY_REPOSITORY}/compare/main...{owner}:{branch}?expand=1\n{opener}\n\
         The registry publishes a collection only once at least 2 members, including a default, have a release."
    );
    Ok(())
}

/// Origin's one push URL, as `release` reads it.
fn origin(dir: &Path, git: &Git) -> Result<String, String> {
    let origin = git.text(dir, ["remote", "get-url", "--push", "--all", "origin"])?;
    if origin.lines().count() != 1 {
        return Err("origin pushes to more than one URL; collection needs exactly one".into());
    }
    Ok(origin)
}

/// The published project IDs and their in-game IDs for origin's repository, ordered by project ID.
fn published_from(
    fetch: Fetch<'_>,
    origin: &str,
    read_index: impl FnOnce() -> Result<Index, String>,
) -> Result<Vec<(String, String)>, String> {
    let name = author_release::github_name(origin)
        .ok_or("origin is not a GitHub repository: the projects a collection lists are found by their GitHub repository")?;
    let id = author_release::github_repository_id(fetch, &name)?;
    let index = read_index()?;
    let mut projects: Vec<_> = index
        .projects
        .iter()
        .filter(|project| {
            project.repository.id == id
                && project
                    .releases
                    .iter()
                    .any(|release| index.revoked(&project.id, &release.file.sha256).is_none())
        })
        .map(|project| (project.id.clone(), project.mod_id.clone()))
        .collect();
    projects.sort_unstable();
    Ok(projects)
}

/// A new `collection.toml`: no `id` yet, `name` and `summary` left for the author, and every
/// project selected by default.
fn scaffold(projects: &[(String, String)]) -> String {
    let mut text = String::from("name = \"\"\nsummary = \"\"\ndescription = \"\"\n");
    if projects.is_empty() {
        text += "member = []\n";
    }
    for (project, mod_id) in projects {
        text += &format!("\n[[member]]\nproject = \"{project}\"  # {mod_id}\n");
    }
    text
}

/// `text` with an `id` added, keeping its formatting and comments, when it has none.
fn with_id(
    text: &str,
    fresh: &dyn Fn() -> Result<String, String>,
) -> Result<Option<(String, String)>, String> {
    let mut document: toml_edit::DocumentMut = text.parse().map_err(|e| format!("{TOML}: {e}"))?;
    if document.contains_key("id") {
        return Ok(None);
    }
    let id = fresh()?;
    document.insert("id", toml_edit::value(id.as_str()));
    // A stable sort: the ID goes first and the other keys keep their order.
    document
        .as_table_mut()
        .sort_values_by(|a, _, b, _| (a.get() != "id").cmp(&(b.get() != "id")));
    Ok(Some((document.to_string(), id)))
}

/// A random collection ID that no project or collection on the registry's `main` has.
fn fresh_id(fetch: Fetch<'_>) -> Result<String, String> {
    const ALPHABET: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    loop {
        let mut id = String::new();
        while id.len() < 8 {
            let mut byte = [0];
            getrandom::fill(&mut byte).map_err(|e| format!("random collection ID: {e}"))?;
            // 248 is the largest multiple of 62 a byte holds: every character is equally likely.
            if byte[0] < 248 {
                id.push(ALPHABET[usize::from(byte[0] % 62)] as char);
            }
        }
        let taken = |path: String| fetch(&format!("{MOD_REGISTRY_METADATA}/{path}"));
        if taken(format!("projects/{id}/project.toml"))?.is_none()
            && taken(format!("collections/{id}/{TOML}"))?.is_none()
        {
            return Ok(id);
        }
    }
}

/// `icon.png`'s bytes when it is a file, read up to one byte past the limit the check refuses.
fn read_icon(path: &Path) -> Result<Option<Vec<u8>>, String> {
    if !path.is_file() {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .and_then(|file| file.take(MAX_ICON as u64 + 1).read_to_end(&mut bytes))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(Some(bytes))
}

/// The registry's checks on the collection, with each member read from the registry's `main`.
/// Returns the logins in `owners` of every member.
fn check(
    fetch: Fetch<'_>,
    collection: &Collection,
    icon: Option<&[u8]>,
) -> Result<Vec<String>, String> {
    if !id(&collection.id) {
        return Err(format!(
            "{TOML}: id {:?} is not 8 characters from 0-9, A-Z and a-z",
            collection.id
        ));
    }
    let mut registered = BTreeMap::new();
    let mut publishers = BTreeMap::new();
    for project in
        std::iter::once(&collection.id).chain(collection.member.iter().map(|m| &m.project))
    {
        if registered.contains_key(project) || !id(project) {
            continue;
        }
        let record = format!("{MOD_REGISTRY_METADATA}/projects/{project}/project.toml");
        let Some(text) = fetch(&record)? else {
            continue;
        };
        let ProjectRecord { owners, repository } =
            toml::from_str(&text).map_err(|e| format!("{record}: {e}"))?;
        let publisher = match publishers.get(&repository) {
            Some(publisher) => publisher,
            None => {
                let file = format!("repositories/{repository}.toml");
                let Repository { name } =
                    toml::from_str(&author_release::registry_file(fetch, &file)?)
                        .map_err(|e| format!("registry/{file}: {e}"))?;
                let owner = name
                    .split_once('/')
                    .map_or(name.as_str(), |(owner, _)| owner)
                    .to_owned();
                publishers.entry(repository).or_insert(owner)
            }
        };
        registered.insert(project.clone(), (publisher.clone(), owners));
    }
    let lookup = |project: &str| {
        registered
            .get(project)
            .map(|(publisher, owners)| Registered {
                publisher: publisher.clone(),
                owners: owners.clone(),
            })
    };
    let report = check_collection(
        &collection.id,
        &[TOML, ICON],
        Some(collection),
        icon,
        &lookup,
    );
    if !report.problems.is_empty() {
        return Err(report
            .problems
            .iter()
            .map(|problem| format!("{}: {}", file_of(&problem.part), problem.message))
            .collect::<Vec<_>>()
            .join("\n"));
    }
    if report.owners.is_empty() {
        return Err(
            "no GitHub account is in owners of every member, so nobody can open the collection's pull request"
                .into(),
        );
    }
    Ok(report.owners)
}

fn file_of(part: &sunrise_mod_package::index::v2::collection::Part) -> &str {
    use sunrise_mod_package::index::v2::collection::Part;
    match part {
        Part::Icon => ICON,
        Part::Entry(name) => name,
        Part::Toml | Part::Folder => TOML,
    }
}

/// Commits `files` into `folder` on top of `upstream`'s `main` and pushes the commit to `branch`
/// on `fork`, replacing what an earlier run pushed there.
fn propose(
    git: &Git,
    upstream: &str,
    fork: &str,
    branch: &str,
    folder: &str,
    files: &[(&str, Vec<u8>)],
    message: &str,
) -> Result<(), String> {
    let scratch = tempfile::tempdir().map_err(|e| format!("collection: {e}"))?;
    let root = scratch.path();
    git.text(root, ["init", "--quiet"])?;
    git.text(
        root,
        ["fetch", "--quiet", "--depth", "1", "--", upstream, "main"],
    )?;
    git.text(root, ["checkout", "--quiet", "--detach", "FETCH_HEAD"])?;
    let target = root.join(folder);
    // An edit replaces the whole folder: a file the registry no longer takes doesn't linger.
    if target.exists() {
        fs::remove_dir_all(&target).map_err(|e| format!("{}: {e}", target.display()))?;
    }
    fs::create_dir_all(&target).map_err(|e| format!("{}: {e}", target.display()))?;
    for (name, bytes) in files {
        fs::write(target.join(name), bytes).map_err(|e| format!("{name}: {e}"))?;
    }
    git.text(root, ["add", "--all", "--", folder])?;
    git.text(
        root,
        ["commit", "--quiet", "--signoff", "--message", message],
    )?;
    git.text(
        root,
        [
            "push",
            "--quiet",
            "--force",
            "--",
            fork,
            &format!("HEAD:refs/heads/{branch}"),
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests;
