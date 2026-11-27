use super::*;
use sunrise_mod_package::index::v2::collection::Member;

/// A registry on `main`: two projects of `example` and one with different owners.
fn registry(url: &str) -> Result<Option<String>, String> {
    let meta = |path: &str| format!("{MOD_REGISTRY_METADATA}/{path}");
    let text = if url == meta("repositories/7.toml") {
        "name = \"example/ports\"\n\n[[package]]\npath = \"doors\"\nproject = \"Ab3dE9xQ\"\n\n[[package]]\npath = \"walls\"\nproject = \"Zz91kLmN\"\n"
    } else if url == meta("repositories/8.toml") {
        "name = \"other/thing\"\n\n[[package]]\npath = \".\"\nproject = \"Ot4erP0j\"\n"
    } else if url == meta("projects/Ab3dE9xQ/project.toml") {
        "mod = \"doors\"\nowners = [\"Example\", \"helper\"]\nrepository = 7\n"
    } else if url == meta("projects/Zz91kLmN/project.toml") {
        "mod = \"walls\"\nowners = [\"example\"]\nrepository = 7\n"
    } else if url == meta("projects/Ot4erP0j/project.toml") {
        "mod = \"thing\"\nowners = [\"other\"]\nrepository = 7\n"
    } else {
        return Ok(None);
    };
    Ok(Some(text.into()))
}

fn png(width: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, width);
        encoder.set_color(png::ColorType::Rgba);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&vec![0; (width * width * 4) as usize])
            .unwrap();
    }
    bytes
}

const ORIGIN: &str = "git@github.com:example/ports.git";

fn origin_id(url: &str) -> Result<Option<String>, String> {
    // Scaffolding may resolve origin but must never read unsigned registry metadata.
    assert_eq!(url, "https://api.github.com/repos/example/ports");
    Ok(Some(r#"{"id":7}"#.into()))
}

#[test]
fn a_new_collection_lists_only_this_repositorys_published_projects_in_id_order() {
    use serde_json::json;

    let digest = "a".repeat(64);
    let release = json!({"version": "1.0.0", "workflow": "release.yml",
        "file": {"url": "https://example.org/mod.smod", "size": 1, "sha256": digest,
            "attestation": "https://example.org/mod.smod.sigstore.json"}});
    let project = |id: &str, mod_id: &str, repo: u64, name: &str, released: bool| {
        json!({
            "id": id, "mod": mod_id, "kind": "mod", "repository": {"id": repo, "name": name},
            "releases": if released { vec![release.clone()] } else { Vec::new() }
        })
    };
    let index = json!({"schema": 2, "serial": 1,
        "projects": [
            project("Zz91kLmN", "walls", 7, "example/old-name", true),
            project("Ot4erP0j", "same_name", 8, "example/ports", true),
            project("NotYet01", "unreleased", 7, "example/ports", false),
            project("AllGone1", "revoked_project", 7, "example/ports", true),
            project("OneGone1", "revoked_release", 7, "example/ports", true),
            project("Ab3dE9xQ", "doors", 7, "example/ports", true)
        ],
        "revoked": [{"project": "AllGone1"}, {"project": "OneGone1", "sha256": digest}]
    });
    let projects = published_from(&origin_id, ORIGIN, || {
        Ok(serde_json::from_value(index).unwrap())
    })
    .unwrap();
    assert_eq!(
        projects,
        [
            ("Ab3dE9xQ".to_owned(), "doors".to_owned()),
            ("Zz91kLmN".to_owned(), "walls".to_owned())
        ]
    );
    let text = scaffold(&projects);
    // The ID is added on the next run, keeping the author's text and comments, and never replaced.
    let (text, id) = with_id(&text, &|| Ok("Qm4tZ81c".into())).unwrap().unwrap();
    assert_eq!(id, "Qm4tZ81c");
    assert!(
        text.starts_with("id = \"Qm4tZ81c\"\nname = \"\"\n"),
        "{text}"
    );
    assert!(text.contains("project = \"Zz91kLmN\"  # walls"), "{text}");
    assert!(
        with_id(&text, &|| panic!("an ID exists"))
            .unwrap()
            .is_none()
    );
    let collection: Collection = toml::from_str(&text).unwrap();
    assert!(collection.member.iter().all(|member| member.default));
    // Its blank name and summary are refused until the author fills them in.
    let error = check(&registry, &collection, Some(&png(16))).unwrap_err();
    assert_eq!(
        error,
        "collection.toml: name must not be blank\ncollection.toml: summary must not be blank"
    );
}

#[test]
fn a_non_github_origin_is_told_apart_from_an_unregistered_github_repository() {
    let no_index = || -> Result<Index, String> { panic!("no index is read without an origin ID") };
    let never = |url: &str| -> Result<Option<String>, String> { panic!("fetched {url}") };
    let elsewhere =
        published_from(&never, "https://gitlab.com/example/ports", no_index).unwrap_err();
    assert!(
        elsewhere.starts_with("origin is not a GitHub repository"),
        "{elsewhere}"
    );
    let unknown = |_: &str| Ok(None);
    let unregistered = published_from(&unknown, ORIGIN, no_index).unwrap_err();
    assert!(
        unregistered.starts_with("This repository isn't registered yet"),
        "{unregistered}"
    );
}

#[test]
fn a_new_collection_can_be_scaffolded_from_the_verified_empty_index() {
    let cache = tempfile::tempdir().unwrap();
    let projects = published_from(&origin_id, ORIGIN, || {
        crate::registry_source::read_first_index(cache.path())
    })
    .unwrap();
    let (text, _) = with_id(&scaffold(&projects), &|| Ok("Qm4tZ81c".into()))
        .unwrap()
        .unwrap();
    let collection: Collection = toml::from_str(&text).unwrap();
    assert!(collection.member.is_empty());
    assert_eq!(
        published_from(&origin_id, ORIGIN, || Err("index unavailable".into())).unwrap_err(),
        "index unavailable"
    );
}

#[test]
fn a_collection_names_who_may_open_it_and_is_refused_when_nobody_owns_every_member() {
    let mut collection = Collection {
        id: "Qm4tZ81c".into(),
        name: "Ports".into(),
        summary: "Doors and walls.".into(),
        description: String::new(),
        member: ["Ab3dE9xQ", "Zz91kLmN"]
            .map(|project| Member {
                project: project.into(),
                default: true,
            })
            .into(),
    };
    let icon = png(16);
    // The existing fixture has legacy [[package]] entries. Name-only and registrant-bearing
    // rows must give the same publisher and owners.
    assert_eq!(
        check(&registry, &collection, Some(&icon)),
        Ok(vec!["Example".to_owned()])
    );
    for row in [
        "name = 'example/ports'\n",
        "name = 'example/ports'\n[[registrant]]\nid = 81\nlogin = 'org-admin'\n[[registrant]]\nid = 82\nlogin = 'another-admin'\n",
    ] {
        let fetch = |url: &str| {
            if url.ends_with("/repositories/7.toml") {
                Ok(Some(row.into()))
            } else {
                registry(url)
            }
        };
        assert_eq!(
            check(&fetch, &collection, Some(&icon)),
            Ok(vec!["Example".to_owned()])
        );
    }
    assert_eq!(
        check(&registry, &collection, None).unwrap_err(),
        "icon.png: expected the collection's PNG icon"
    );
    collection.member[1].project = "Ot4erP0j".into();
    assert_eq!(
        check(&registry, &collection, Some(&png(16))).unwrap_err(),
        "no GitHub account is in owners of every member, so nobody can open the collection's pull request"
    );
    collection.member[1].project = "Nope0000".into();
    assert_eq!(
        check(&registry, &collection, Some(&png(16))).unwrap_err(),
        "collection.toml: member Nope0000 is not a registered project"
    );
}

#[cfg(unix)]
mod git_fixtures {
    use super::*;

    fn git(dir: &Path, args: &[&str]) -> String {
        Git::default().text(dir, args).unwrap()
    }

    #[test]
    #[ignore = "daily"]
    fn a_proposal_is_one_signed_off_commit_on_main_pushed_to_the_forks_branch() {
        let temp = tempfile::tempdir().unwrap();
        let upstream = temp.path().join("upstream");
        fs::create_dir_all(upstream.join("registry/collections/Qm4tZ81c")).unwrap();
        fs::write(
            upstream.join("registry/collections/Qm4tZ81c/old.txt"),
            "old",
        )
        .unwrap();
        git(&upstream, &["init", "--quiet", "--initial-branch", "main"]);
        for (key, value) in [
            ("user.name", "Registry"),
            ("user.email", "registry@example.com"),
            ("commit.gpgsign", "false"),
        ] {
            git(&upstream, &["config", key, value]);
        }
        git(&upstream, &["add", "--all"]);
        git(&upstream, &["commit", "--quiet", "--message", "registry"]);
        let fork = temp.path().join("fork.git");
        git(
            temp.path(),
            &[
                "clone",
                "--quiet",
                "--bare",
                upstream.to_str().unwrap(),
                fork.to_str().unwrap(),
            ],
        );
        // The author's identity comes from their own Git configuration.
        let home = temp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        fs::write(
            home.join(".gitconfig"),
            "[user]\n\tname = Example\n\temail = example@example.com\n[commit]\n\tgpgsign = false\n",
        )
        .unwrap();
        let author = Git {
            env: vec![
                ("HOME", home.clone().into()),
                ("GIT_CONFIG_NOSYSTEM", "1".into()),
            ],
        };
        let upstream_url = format!("file://{}", upstream.display());
        let propose_once = |text: &str| {
            propose(
                &author,
                &upstream_url,
                fork.to_str().unwrap(),
                "collection-Qm4tZ81c",
                "registry/collections/Qm4tZ81c",
                &[(TOML, text.as_bytes().to_vec()), (ICON, png(16))],
                "Update the Ports collection",
            )
        };
        propose_once("first").unwrap();
        // Running again replaces the branch rather than stacking on it.
        propose_once("second").unwrap();
        let branch = "collection-Qm4tZ81c";
        assert_eq!(
            git(&fork, &["rev-list", "--count", branch]),
            "2",
            "one commit on top of main"
        );
        assert_eq!(
            git(
                &fork,
                &[
                    "ls-tree",
                    "--name-only",
                    &format!("{branch}:registry/collections/Qm4tZ81c")
                ]
            ),
            "collection.toml\nicon.png"
        );
        assert_eq!(
            git(
                &fork,
                &[
                    "show",
                    &format!("{branch}:registry/collections/Qm4tZ81c/collection.toml")
                ]
            ),
            "second"
        );
        let body = git(&fork, &["log", "-1", "--format=%an%n%B", branch]);
        assert!(
            body.starts_with("Example\nUpdate the Ports collection"),
            "{body}"
        );
        assert!(
            body.contains("Signed-off-by: Example <example@example.com>"),
            "{body}"
        );
    }
}
