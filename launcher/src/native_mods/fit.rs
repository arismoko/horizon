//! Whether a native mod made for one game build fits another (SDK design §10): an API-only mod
//! (no `[native.*]` table, no internals) is built against any build where every item it uses is
//! unchanged, and only the built binary is exact to the build. The mod's uses table
//! ([`uses::USES_FILE`], written by `cargo horizon-mod build`) says what it uses, each item with
//! its fingerprint at the build it was made for; the build's `surface.json` says what is there
//! now; `mod_surface::uses::fits` compares the two. The compile against the build stays the final
//! check, and the loader's build-id check is unchanged.
//!
//! People read "made for" (a mod's pin) and "fits"; builds by their Nightly names; and items as
//! what they are ("the hook `player_logged_in`"), never surface ids or hashes.
use super::{api, internals, prebuilt, sdk_cache};
use crate::i18n::{tr, trf};
use horizon_mod_package::mod_surface;
use mod_surface::uses::{self, Change, Fit, ItemKind, Misfit, UsesTable};
use std::io::Read;
use std::path::Path;

/// At most this many changed items are named in a message; the build log has them all.
pub const SHOWN_ITEMS: usize = 5;

/// How a mod stands against the build an instance plays.
#[derive(Clone, Debug, PartialEq)]
pub enum Verdict {
    /// Made for this build.
    MadeFor,
    /// Made for another build (`made_for`), and every item it uses is unchanged in this one.
    Fits { made_for: String },
    /// It changes game code or names the game's internals (`why`: the table that does, or
    /// `internals`): it works on the build it was made for only.
    Exact { made_for: String, why: &'static str },
    /// Its package doesn't say what it uses (made by an older tool, or its list is for another
    /// build than its pin): it works on the build it was made for only.
    NoList { made_for: String },
    /// Items it uses changed in this build.
    DoesNotFit {
        made_for: String,
        misfits: Vec<Misfit>,
    },
    /// An `[[inject]]` row's point is not the same in this build: why.
    Points { made_for: String, why: String },
}

impl Verdict {
    /// Whether the mod can be built against this build.
    pub fn usable(&self) -> bool {
        matches!(self, Self::MadeFor | Self::Fits { .. })
    }
}

/// The uses table `dir` carries, when it carries one this launcher reads. A table that is too
/// large, of another schema or malformed is no table: the mod then works on its own build only.
pub(super) fn table(dir: &Path) -> Option<UsesTable> {
    let file = std::fs::File::open(dir.join(uses::USES_FILE)).ok()?;
    let mut bytes = Vec::new();
    file.take(uses::MAX_USES_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    UsesTable::read(&bytes).ok()
}

/// How `project` stands against build `build`, whose surface `surface` gives (asked for only when
/// the mod was made for another build) and whose build data `catalog` holds.
pub(super) fn judge(
    project: &api::Project<'_>,
    build: &str,
    surface: &mut dyn FnMut() -> Result<std::rc::Rc<mod_surface::Surface>, sdk_cache::Refusal>,
    catalog: &mut dyn FnMut() -> Result<std::rc::Rc<internals::Catalog>, sdk_cache::Refusal>,
) -> Result<Verdict, sdk_cache::Refusal> {
    let made_for = project.bundle.native().horizon.clone();
    if made_for == build {
        return Ok(Verdict::MadeFor);
    }
    if let Some(why) = prebuilt::changes_game(project) {
        return Ok(Verdict::Exact { made_for, why });
    }
    let Some(table) = table(&project.bundle.dir).filter(|table| table.made_for == made_for) else {
        return Ok(Verdict::NoList { made_for });
    };
    let target = surface()?;
    if let Fit::DoesNotFit(misfits) = uses::fits(&table.entries, target.as_ref()) {
        return Ok(Verdict::DoesNotFit { made_for, misfits });
    }
    if !project.bundle.manifest.inject.is_empty() {
        if let Err(error) = catalog()?.check(&project.bundle.manifest) {
            return Ok(Verdict::Points {
                made_for,
                why: error.to_string(),
            });
        }
    }
    Ok(Verdict::Fits { made_for })
}

/// An item as people read it: "the hook `player_logged_in`".
pub fn item(id: &str) -> String {
    let Some((kind, name)) = uses::item(id) else {
        return format!("`{id}`");
    };
    let en = match kind {
        ItemKind::Hook => "the hook `{name}`",
        ItemKind::Argument => "the argument `{name}`",
        ItemKind::Trait => "the trait `{name}`",
        ItemKind::Method => "the method `{name}`",
        ItemKind::AssociatedItem => "the associated item `{name}`",
        ItemKind::Function => "the function `{name}`",
        ItemKind::Macro => "the macro `{name}`",
        ItemKind::PartKind => "the Part kind `{name}`",
        ItemKind::Capability => "the capability `{name}`",
        ItemKind::Type => "the type `{name}`",
        ItemKind::Variant => "the variant `{name}`",
        ItemKind::Field => "the field `{name}`",
    };
    trf(en, &[("name", &name)])
}

/// What changed about one item, as people read it: "the hook `player_logged_in` (changed)".
pub fn change(misfit: &Misfit) -> String {
    let what = item(&misfit.id);
    let leaf = |id: &str| mod_surface::leaf(id).to_owned();
    match &misfit.change {
        Change::Removed => trf("{item} (removed)", &[("item", &what)]),
        Change::Changed => trf("{item} (changed)", &[("item", &what)]),
        Change::LostImpl(name) => trf(
            "{item} no longer implements `{trait}`",
            &[("item", &what), ("trait", name)],
        ),
        Change::Fields { gained, at } if gained.is_empty() => trf(
            "the fields of {item} changed, and `{at}` builds it field by field",
            &[("item", &what), ("at", at)],
        ),
        Change::Fields { gained, at } => trf(
            "{item} gained {fields}, and `{at}` builds it field by field",
            &[
                ("item", &what),
                (
                    "fields",
                    &list(gained.iter().map(|name| format!("`{name}`"))),
                ),
                ("at", at),
            ],
        ),
        Change::Version { named, now } => trf(
            "{item} is now version {now}; this mod names version {named}",
            &[
                ("item", &what),
                ("now", &now.to_string()),
                ("named", &named.to_string()),
            ],
        ),
        Change::NotBuildable { at } => trf(
            "{item} can no longer be built with a literal, and `{at}` builds it",
            &[("item", &what), ("at", at)],
        ),
        Change::Path(path) => trf(
            "`mod_api::{path}` is no longer exported under that path",
            &[("path", path)],
        ),
        Change::Route { from, owner } => trf(
            "`{from}` no longer includes `{owner}`, which this mod calls `{member}` on",
            &[
                ("from", &leaf(from)),
                ("owner", &leaf(owner)),
                ("member", &leaf(&misfit.id)),
            ],
        ),
    }
}

/// Words joined as a list: "a", "a and b", "a, b and c".
fn list(words: impl Iterator<Item = String>) -> String {
    let words: Vec<String> = words.collect();
    match words.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => trf(
            "{first} and {last}",
            &[("first", &rest.join(", ")), ("last", last)],
        ),
    }
}

/// The changed items of `misfits`, at most [`SHOWN_ITEMS`], then how many more.
pub fn changes(misfits: &[Misfit]) -> String {
    let shown: Vec<String> = misfits.iter().take(SHOWN_ITEMS).map(change).collect();
    let more = misfits.len().saturating_sub(SHOWN_ITEMS);
    let shown = shown.join("; ");
    match more {
        0 => shown,
        more => trf(
            "{items}; and {count} more (all of them are in the build log)",
            &[("items", &shown), ("count", &more.to_string())],
        ),
    }
}

/// The line a mod's row shows about `verdict` for mod `name` in an instance on `build`
/// (as people read it).
pub fn line(name: &str, verdict: &Verdict, build: &str) -> String {
    let made = |commit: &str| sdk_cache::known_name(commit);
    match verdict {
        Verdict::MadeFor => trf("Made for this game ({build}).", &[("build", build)]),
        Verdict::Fits { made_for } => trf(
            "Made for {made_for}. Fits this game ({build}).",
            &[("made_for", &made(made_for)), ("build", build)],
        ),
        Verdict::Exact { made_for, why } if *why == "internals" => trf(
            "Made for {made_for} only: it uses the game's internals, so it works on exactly that build.",
            &[("made_for", &made(made_for))],
        ),
        Verdict::Exact { made_for, why } => trf(
            "Made for {made_for} only: it changes game code ({table}), so it works on exactly that build.",
            &[("made_for", &made(made_for)), ("table", why)],
        ),
        Verdict::NoList { made_for } => trf(
            "{mod}'s package was made by an older tool and doesn't say what it uses, so it works on {made_for} only. Its author can rebuild it with the current cargo horizon-mod.",
            &[("mod", name), ("made_for", &made(made_for))],
        ),
        Verdict::DoesNotFit { made_for, misfits } => {
            let count = misfits.len().to_string();
            if misfits.len() == 1 {
                trf(
                    "It was made for {made_for} and this instance runs {build}. Since then the game changed one thing {mod} uses: {items}.",
                    &[
                        ("made_for", &made(made_for)),
                        ("build", build),
                        ("mod", name),
                        ("items", &changes(misfits)),
                    ],
                )
            } else {
                trf(
                    "It was made for {made_for} and this instance runs {build}. Since then the game changed {count} things {mod} uses: {items}.",
                    &[
                        ("made_for", &made(made_for)),
                        ("build", build),
                        ("count", &count),
                        ("mod", name),
                        ("items", &changes(misfits)),
                    ],
                )
            }
        }
        Verdict::Points { made_for, why } => trf(
            "It was made for {made_for} and this instance runs {build}, where a point it hooks is not the same: {why}",
            &[("made_for", &made(made_for)), ("build", build), ("why", why)],
        ),
    }
}

/// The headline over a verdict the build can't use: "Greeter doesn't fit this game."
pub fn headline(name: &str, verdict: &Verdict) -> String {
    match verdict {
        Verdict::MadeFor | Verdict::Fits { .. } => trf("{mod} fits this game.", &[("mod", name)]),
        Verdict::Exact { .. } | Verdict::NoList { .. } => {
            trf("{mod} works on another game build only.", &[("mod", name)])
        }
        Verdict::DoesNotFit { .. } | Verdict::Points { .. } => {
            trf("{mod} doesn't fit this game.", &[("mod", name)])
        }
    }
}

/// What the player can do about a mod the build can't use: only what a button does today (the
/// mod's menu on the Mods page has Turn off).
pub fn actions(name: &str) -> String {
    trf(
        "What you can do: play without {mod}. It stays installed and is left out of the build; Turn off {mod} in its menu on the Mods page to keep it out.",
        &[("mod", name)],
    )
}

/// Why a mod the build can't use won't be built, in a few words: what changed, or why it is
/// tied to its own build.
pub fn short(verdict: &Verdict) -> String {
    match verdict {
        Verdict::MadeFor | Verdict::Fits { .. } => String::new(),
        Verdict::Exact { why, .. } if *why == "internals" => {
            tr("it uses the game's internals").to_owned()
        }
        Verdict::Exact { why, .. } => trf("it changes game code ({table})", &[("table", why)]),
        Verdict::NoList { .. } => tr("its package doesn't say what it uses").to_owned(),
        Verdict::DoesNotFit { misfits, .. } => changes(misfits),
        Verdict::Points { why, .. } => why.clone(),
    }
}

/// What updating to `build` does to the instance's `total` native mods, said before anything is
/// downloaded (design-fit §10.1): how many fit, and each one that will be switched off (id, why).
pub fn update_summary(build: &str, total: usize, off: &[(String, String)]) -> String {
    if off.is_empty() {
        return if total == 1 {
            trf(
                "Update to {build}? Your native mod fits it.",
                &[("build", build)],
            )
        } else {
            trf(
                "Update to {build}? All {count} of your native mods fit it.",
                &[("build", build), ("count", &total.to_string())],
            )
        };
    }
    let head = trf(
        "Update to {build}? {fit} of your {count} native mods fit it.",
        &[
            ("build", build),
            ("fit", &total.saturating_sub(off.len()).to_string()),
            ("count", &total.to_string()),
        ],
    );
    let lines: Vec<String> = off
        .iter()
        .map(|(name, why)| {
            trf(
                "{mod} does not: {why}. It will be switched off until it is updated.",
                &[("mod", name), ("why", why)],
            )
        })
        .collect();
    format!("{head}\n{}", lines.join("\n"))
}

/// Said under an update's summary when worlds were saved with a mod it switches off.
pub fn update_worlds(worlds: &[String]) -> String {
    let named = list(worlds.iter().map(|world| format!("“{world}”")));
    if worlds.len() == 1 {
        trf(
            "One of your worlds uses a mod this update switches off: {worlds}. Opening it without that mod removes its blocks and what they hold, for good.",
            &[("worlds", &named)],
        )
    } else {
        trf(
            "{count} of your worlds use a mod this update switches off: {worlds}. Opening them without it removes its blocks and what they hold, for good.",
            &[("count", &worlds.len().to_string()), ("worlds", &named)],
        )
    }
}

/// The button that takes the update.
pub fn update() -> &'static str {
    tr("Update")
}

/// The whole message for mod `name` that the build can't use: headline, why, and what to do.
pub fn refusal(name: &str, verdict: &Verdict, build: &str) -> String {
    format!(
        "{}\n{}\n{}",
        headline(name, verdict),
        line(name, verdict, build),
        actions(name)
    )
}

/// The message after a mod judged to fit didn't compile.
pub fn did_not_build(name: &str) -> String {
    trf(
        "{mod} looked compatible with this game but didn't build. This is for {mod}'s author to fix; the mod is switched off here. Details were saved to the build log.",
        &[("mod", name)],
    )
}

/// What is said when a mod the build can't use was saved into worlds: the worlds named, and
/// that opening them without the mod removes its content for good. `build` is the build the
/// mod doesn't fit.
pub fn worlds_at_stake(name: &str, build: &str, worlds: &[String]) -> String {
    let headline = trf(
        "{mod} doesn't fit {build}.",
        &[("mod", name), ("build", build)],
    );
    format!("{headline}\n{}", worlds_body(name, worlds))
}

/// [`worlds_at_stake`] for a mod judged to fit `build` whose compile against it failed.
pub fn worlds_at_stake_unbuilt(name: &str, build: &str, worlds: &[String]) -> String {
    let headline = trf(
        "{mod} looked compatible with {build} but didn't build.",
        &[("mod", name), ("build", build)],
    );
    format!("{headline}\n{}", worlds_body(name, worlds))
}

/// The worlds saved with mod `name`, and what opening them without it does.
fn worlds_body(name: &str, worlds: &[String]) -> String {
    let named = list(worlds.iter().map(|world| format!("“{world}”")));
    if worlds.len() == 1 {
        trf(
            "One of your worlds uses {mod}: {worlds}. Opening it without {mod} removes its blocks and what they hold, for good.",
            &[("mod", name), ("worlds", &named)],
        )
    } else {
        trf(
            "{count} of your worlds use {mod}: {worlds}. Opening them without {mod} removes its blocks and what they hold, for good.",
            &[
                ("count", &worlds.len().to_string()),
                ("mod", name),
                ("worlds", &named),
            ],
        )
    }
}

/// The choice that keeps everything as it is, said on its button.
pub fn stay(current: &str) -> String {
    trf("Stay on {build}", &[("build", current)])
}

/// The other choice, said on its button.
pub fn back_up_and_update() -> &'static str {
    tr("Back up these worlds and update")
}

#[cfg(test)]
#[path = "fit_tests.rs"]
mod tests;
