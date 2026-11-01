//! Resolve bundle dependencies without touching the staged workspace.
use semver::{Version, VersionReq};
use std::collections::{BTreeMap, HashMap};
use std::fmt;

#[derive(Debug)]
pub(super) enum Error {
    Manifest {
        owner: String,
        target: String,
        reason: String,
    },
    Duplicate(String),
    Missing {
        owner: String,
        target: String,
    },
    NoApi {
        owner: String,
        target: String,
    },
    Version {
        owner: String,
        target: String,
        version: String,
        requirement: String,
    },
    Cycle {
        mods: Vec<String>,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Manifest {
                owner,
                target,
                reason,
            } => write!(f, "{owner}: dependency {target}: {reason}"),
            Self::Duplicate(id) => write!(f, "Duplicate native mod id {id:?}"),
            Self::Missing { owner, target } => write!(f, "{owner} requires missing mod {target}"),
            Self::NoApi { owner, target } => {
                write!(f, "{owner} depends on {target}, which has no [api] package")
            }
            Self::Version {
                owner,
                target,
                version,
                requirement,
            } => write!(
                f,
                "{owner} requires {target} {requirement}, but installed version is {version}"
            ),
            Self::Cycle { mods } => write!(f, "Mod dependency cycle: {}", mods.join(" -> ")),
        }
    }
}

#[derive(Debug)]
pub(super) struct Dependency {
    pub id: String,
    pub requirement: VersionReq,
    pub optional: bool,
}

pub(super) fn parse(owner: &str, value: Option<&toml::Value>) -> Result<Vec<Dependency>, Error> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let invalid = |target: &str, reason: &str| Error::Manifest {
        owner: owner.into(),
        target: target.into(),
        reason: reason.into(),
    };
    let table = value
        .as_table()
        .ok_or_else(|| invalid("[dependencies]", "expected a table"))?;
    table
        .iter()
        .map(|(id, value)| {
            if !super::api::valid_id(id) {
                return Err(invalid(id, "invalid mod id"));
            }
            let (version, optional) = if let Some(version) = value.as_str() {
                (version, false)
            } else if let Some(details) = value.as_table() {
                if details
                    .keys()
                    .any(|key| key != "version" && key != "optional")
                {
                    return Err(invalid(id, "only version and optional are accepted"));
                }
                let version = details
                    .get("version")
                    .and_then(toml::Value::as_str)
                    .ok_or_else(|| invalid(id, "version must be a string"))?;
                let optional = match details.get("optional") {
                    Some(value) => value
                        .as_bool()
                        .ok_or_else(|| invalid(id, "optional must be a boolean"))?,
                    None => false,
                };
                (version, optional)
            } else {
                return Err(invalid(id, "expected a version string or table"));
            };
            let requirement =
                VersionReq::parse(version).map_err(|e| invalid(id, &e.to_string()))?;
            Ok(Dependency {
                id: id.clone(),
                requirement,
                optional,
            })
        })
        .collect()
}

pub(super) struct Mod<'a> {
    pub id: &'a str,
    pub version: &'a str,
    pub has_api: bool,
    pub dependencies: &'a [Dependency],
}

pub(super) struct Resolution {
    /// Original bundle indices, in dispatch order.
    pub order: Vec<usize>,
    /// Installed dependency indices for each original bundle index.
    pub edges: Vec<Vec<usize>>,
}

pub(super) fn resolve(mods: &[Mod<'_>]) -> Result<Resolution, Error> {
    let mut ids = HashMap::new();
    for (index, item) in mods.iter().enumerate() {
        if ids.insert(item.id, index).is_some() {
            return Err(Error::Duplicate(item.id.into()));
        }
    }
    let mut edges = vec![Vec::new(); mods.len()];
    for (index, item) in mods.iter().enumerate() {
        for dep in item.dependencies {
            let Some(&target) = ids.get(dep.id.as_str()) else {
                if dep.optional {
                    continue;
                }
                return Err(Error::Missing {
                    owner: item.id.into(),
                    target: dep.id.clone(),
                });
            };
            let other = &mods[target];
            if !item.has_api || !other.has_api {
                return Err(Error::NoApi {
                    owner: item.id.into(),
                    target: dep.id.clone(),
                });
            }
            let version = Version::parse(other.version).map_err(|e| Error::Manifest {
                owner: item.id.into(),
                target: dep.id.clone(),
                reason: format!("invalid installed version {:?}: {e}", other.version),
            })?;
            if !dep.requirement.matches(&version) {
                return Err(Error::Version {
                    owner: item.id.into(),
                    target: dep.id.clone(),
                    version: other.version.into(),
                    requirement: dep.requirement.to_string(),
                });
            }
            edges[index].push(target);
        }
    }
    let mut done = vec![false; mods.len()];
    let mut order = Vec::with_capacity(mods.len());
    while order.len() < mods.len() {
        let next = (0..mods.len()).find(|&i| !done[i] && edges[i].iter().all(|&d| done[d]));
        let Some(next) = next else {
            // Follow remaining edges until a node repeats, reporting the actual cycle only.
            let mut seen = BTreeMap::new();
            let mut path = Vec::new();
            let mut current = done.iter().position(|done| !done).expect("unfinished node");
            loop {
                if let Some(&start) = seen.get(&current) {
                    let mut cycle: Vec<String> = path[start..]
                        .iter()
                        .map(|&i| mods[i].id.to_owned())
                        .collect();
                    cycle.push(mods[current].id.to_owned());
                    return Err(Error::Cycle { mods: cycle });
                }
                seen.insert(current, path.len());
                path.push(current);
                current = *edges[current]
                    .iter()
                    .find(|&&i| !done[i])
                    .expect("unfinished dependency");
            }
        };
        done[next] = true;
        order.push(next);
    }
    Ok(Resolution { order, edges })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn deps(text: &str) -> Vec<Dependency> {
        let value: toml::Value = text.parse().unwrap();
        parse("user", value.get("dependencies")).unwrap()
    }
    fn item<'a>(id: &'a str, version: &'a str, dependencies: &'a [Dependency]) -> Mod<'a> {
        Mod {
            id,
            version,
            has_api: true,
            dependencies,
        }
    }
    #[test]
    fn order_and_ties_include_present_optional() {
        let user = deps("[dependencies]\nbase = '^1'\nextra = { version = '^2', optional = true }");
        let mods = [
            item("user", "1.0.0", &user),
            item("base", "1.2.0", &[]),
            item("extra", "2.0.0", &[]),
            item("other", "1.0.0", &[]),
        ];
        let result = resolve(&mods).unwrap();
        assert_eq!(result.order, [1, 2, 0, 3]);
        assert_eq!(result.edges[0], [1, 2]);
        assert_eq!(
            resolve(&[item("user", "1.0.0", &user), item("base", "1.2.0", &[])])
                .unwrap()
                .order,
            [1, 0]
        );
    }
    #[test]
    fn failures_name_mods() {
        let required = deps("[dependencies]\nbase = '^1'");
        let optional = deps("[dependencies]\nbase = { version = '^1', optional = true }");
        for dependencies in [&required, &optional] {
            let err = resolve(&[
                item("user", "1.0.0", dependencies),
                item("base", "2.0.0", &[]),
            ])
            .err()
            .unwrap()
            .to_string();
            assert!(err.contains("user") && err.contains("base"));
        }
        let err = resolve(&[item("user", "1.0.0", &required)])
            .err()
            .unwrap()
            .to_string();
        assert!(err.contains("user") && err.contains("base"));
        let back = deps("[dependencies]\nuser = '^1'");
        let err = resolve(&[
            item("user", "1.0.0", &required),
            item("base", "1.0.0", &back),
        ])
        .err()
        .unwrap()
        .to_string();
        assert!(err.contains("user") && err.contains("base"));
    }
    #[test]
    fn malformed_and_no_api() {
        let value: toml::Value = "[dependencies]\nbase = { version = '^1', optional = 'yes' }"
            .parse()
            .unwrap();
        assert!(parse("user", value.get("dependencies")).is_err());
        let required = deps("[dependencies]\nbase = '^1'");
        let mut base = item("base", "1.0.0", &[]);
        base.has_api = false;
        assert!(matches!(
            resolve(&[item("user", "1.0.0", &required), base]),
            Err(Error::NoApi { .. })
        ));
    }
}
