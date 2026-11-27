//! A registry collection, `registry/collections/<id>/` in sunrise-org/sunrise-mods
//! (`docs/registry-v2.md`): one publisher's mods shown as one card. The registry's checker and
//! `cargo sunrise-mod collection` both judge a collection with [`check_collection`].
use crate::icon::Icon;
use serde::{Deserialize, Serialize};

/// The collection's file, in its folder.
pub const TOML: &str = "collection.toml";
/// The collection's icon, in its folder, under the rules of a mod's icon.
pub const ICON: &str = "icon.png";

#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Collection {
    /// The permanent collection ID, which names the folder.
    pub id: String,
    pub name: String,
    pub summary: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub member: Vec<Member>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Member {
    /// The member's project ID.
    pub project: String,
    /// Whether the member is selected when a player opens the collection.
    #[serde(default = "selected")]
    pub default: bool,
}

fn selected() -> bool {
    true
}

/// What the checks read of a registered project.
pub struct Registered {
    /// The GitHub account that owns the repository the project is registered to.
    pub publisher: String,
    /// The GitHub logins in the project's `owners`.
    pub owners: Vec<String>,
}

/// Where in the folder a problem is.
#[derive(Debug, PartialEq)]
pub enum Part {
    Folder,
    Toml,
    Icon,
    /// A file or folder that has no place in a collection's folder.
    Entry(String),
}

#[derive(Debug, PartialEq)]
pub struct Problem {
    pub part: Part,
    pub message: String,
}

/// The outcome of [`check_collection`].
#[derive(Debug, Default)]
pub struct Report {
    pub problems: Vec<Problem>,
    /// The account every registered member is published by, the first member's when they differ.
    pub publisher: Option<String>,
    /// The logins in `owners` of every member, as the first member spells them: who may open a
    /// pull request that changes the collection.
    pub owners: Vec<String>,
}

/// Checks a collection's folder. `folder` is the folder's name and `entries` the name of every
/// file and folder in it. `collection` is its parsed `collection.toml`, or `None` when that is
/// missing or unreadable, which the caller reports. `icon` is the bytes of `icon.png` when it is a
/// file, read up to one byte past [`crate::icon::MAX_ICON`]. `lookup` finds a registered project
/// by ID.
pub fn check_collection(
    folder: &str,
    entries: &[&str],
    collection: Option<&Collection>,
    icon: Option<&[u8]>,
    lookup: &dyn Fn(&str) -> Option<Registered>,
) -> Report {
    let mut problems = Vec::new();
    let mut problem = |part, message: &str| {
        problems.push(Problem {
            part,
            message: message.to_owned(),
        })
    };
    if !id(folder) {
        problem(
            Part::Folder,
            "expected a directory named by an 8-character base62 collection ID",
        );
    }
    for entry in entries.iter().filter(|e| ![TOML, ICON].contains(e)) {
        problem(
            Part::Entry((*entry).to_owned()),
            "unexpected registry entry",
        );
    }
    match icon.map(|bytes| Icon::parse(bytes.to_vec())) {
        Some(Ok(_)) => {}
        Some(Err(error)) => problem(Part::Icon, &error),
        None => problem(Part::Icon, "expected the collection's PNG icon"),
    }
    let mut publisher: Option<String> = None;
    let mut owners: Option<Vec<String>> = None;
    if let Some(collection) = collection {
        if collection.id != folder {
            problem(Part::Toml, "collection ID must match the directory name");
        }
        if lookup(&collection.id).is_some() {
            problem(Part::Toml, "collection ID is the ID of a project");
        }
        for (field, text) in [("name", &collection.name), ("summary", &collection.summary)] {
            if text.trim().is_empty() {
                problem(Part::Toml, &format!("{field} must not be blank"));
            }
        }
        if collection.member.len() < 2 {
            problem(Part::Toml, "a collection needs at least two members");
        }
        if !collection.member.iter().any(|member| member.default) {
            problem(
                Part::Toml,
                "at least one member must be selected by default",
            );
        }
        for (at, member) in collection.member.iter().enumerate() {
            let project = &member.project;
            if collection.member[..at]
                .iter()
                .any(|m| m.project == *project)
            {
                problem(Part::Toml, &format!("duplicate member {project}"));
            }
            let Some(registered) = lookup(project) else {
                problem(
                    Part::Toml,
                    &format!("member {project} is not a registered project"),
                );
                continue;
            };
            match &publisher {
                None => publisher = Some(registered.publisher),
                // GitHub logins are case-insensitive.
                Some(first) if !first.eq_ignore_ascii_case(&registered.publisher) => problem(
                    Part::Toml,
                    &format!(
                        "member {project} is published by {}, not by {first}: a collection lists one publisher's mods",
                        registered.publisher
                    ),
                ),
                Some(_) => {}
            }
            match &mut owners {
                None => owners = Some(registered.owners),
                Some(common) => common.retain(|login| {
                    registered
                        .owners
                        .iter()
                        .any(|owner| owner.eq_ignore_ascii_case(login))
                }),
            }
        }
    }
    Report {
        problems,
        publisher,
        owners: owners.unwrap_or_default(),
    }
}

/// A collection ID: eight case-sensitive base62 characters, the form of a project ID.
pub fn id(text: &str) -> bool {
    text.len() == 8 && text.bytes().all(|b| b.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests;
