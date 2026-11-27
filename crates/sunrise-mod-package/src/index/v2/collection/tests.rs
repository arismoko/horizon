use super::*;

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&vec![0; (width * height * 4) as usize])
            .unwrap();
    }
    bytes
}

/// Three projects of `example`, one of them co-owned by `Helper`, and one of `other`.
fn lookup(project: &str) -> Option<Registered> {
    let (publisher, owners): (&str, &[&str]) = match project {
        "Ab3dE9xQ" => ("example", &["Example", "helper"]),
        "Zz91kLmN" => ("Example", &["example", "Helper"]),
        "Cd5fG1zS" => ("example", &["example"]),
        "Ot4erP0j" => ("other", &["other"]),
        _ => return None,
    };
    Some(Registered {
        publisher: publisher.into(),
        owners: owners.iter().map(|o| (*o).into()).collect(),
    })
}

const GOOD: &str = r#"
id = "Qm4tZ81c"
name = "Wayfinding"
summary = "The map and its markers."

[[member]]
project = "Ab3dE9xQ"

[[member]]
project = "Zz91kLmN"
default = false
"#;

fn judge(toml: &str) -> Report {
    let collection: Collection = ::toml::from_str(toml).unwrap();
    check_collection(
        "Qm4tZ81c",
        &[TOML, ICON],
        Some(&collection),
        Some(&png(16, 16)),
        &lookup,
    )
}

fn messages(report: &Report) -> Vec<&str> {
    report.problems.iter().map(|p| p.message.as_str()).collect()
}

#[test]
fn a_collection_of_one_publisher_names_it_and_the_owners_of_every_member() {
    let report = judge(GOOD);
    assert_eq!(messages(&report), Vec::<&str>::new());
    assert_eq!(report.publisher.as_deref(), Some("example"));
    // Logins compare without case, spelled as the first member spells them.
    assert_eq!(report.owners, ["Example", "helper"]);
    let three = judge(&format!("{GOOD}\n[[member]]\nproject = \"Cd5fG1zS\"\n"));
    assert_eq!(three.owners, ["Example"]);
}

#[test]
fn each_rule_refuses_with_its_reason() {
    for (from, to, expected) in [
        (
            "id = \"Qm4tZ81c\"",
            "id = \"Qm4tZ81d\"",
            "collection ID must match the directory name",
        ),
        (
            "name = \"Wayfinding\"",
            "name = \" \"",
            "name must not be blank",
        ),
        (
            "summary = \"The map and its markers.\"",
            "summary = \"\"",
            "summary must not be blank",
        ),
        (
            "[[member]]\nproject = \"Zz91kLmN\"\ndefault = false\n",
            "",
            "a collection needs at least two members",
        ),
        (
            "project = \"Ab3dE9xQ\"",
            "project = \"Ab3dE9xQ\"\ndefault = false",
            "at least one member must be selected by default",
        ),
        ("Zz91kLmN", "Ab3dE9xQ", "duplicate member Ab3dE9xQ"),
        (
            "Zz91kLmN",
            "Nope0000",
            "member Nope0000 is not a registered project",
        ),
        (
            "Zz91kLmN",
            "Ot4erP0j",
            "member Ot4erP0j is published by other, not by example: a collection lists one publisher's mods",
        ),
    ] {
        let toml = GOOD.replace(from, to);
        assert_ne!(toml, GOOD, "{from}");
        assert_eq!(messages(&judge(&toml)), [expected], "{from} -> {to}");
    }
}

#[test]
fn a_collection_without_members_is_read_and_refused_by_the_member_rules() {
    let toml = "id = \"Qm4tZ81c\"\nname = \"Wayfinding\"\nsummary = \"The map and its markers.\"\n";
    assert_eq!(
        messages(&judge(toml)),
        [
            "a collection needs at least two members",
            "at least one member must be selected by default"
        ]
    );
}

#[test]
fn the_folder_holds_its_file_and_a_square_png_icon_and_is_named_by_a_free_id() {
    let collection: Collection = ::toml::from_str(GOOD).unwrap();
    let check = |folder, entries: &[&str], icon: Option<&[u8]>| {
        let report = check_collection(folder, entries, Some(&collection), icon, &lookup);
        report.problems
    };
    let icon = png(16, 16);
    assert_eq!(
        check("Qm4tZ81c", &[TOML, ICON, "notes.txt"], Some(&icon)),
        [Problem {
            part: Part::Entry("notes.txt".into()),
            message: "unexpected registry entry".into()
        }]
    );
    assert_eq!(
        check("Qm4tZ81c", &[TOML], None),
        [Problem {
            part: Part::Icon,
            message: "expected the collection's PNG icon".into()
        }]
    );
    assert_eq!(
        check("Qm4tZ81c", &[TOML, ICON], Some(&png(16, 8)))[0].message,
        "icon.png must be square and at most 512×512"
    );
    let named = check("Qm4tZ81", &[TOML, ICON], Some(&icon));
    assert_eq!(named[0].part, Part::Folder);
    // A project's ID is never a collection's.
    let taken = GOOD.replace("Qm4tZ81c", "Cd5fG1zS");
    let collection: Collection = ::toml::from_str(&taken).unwrap();
    let report = check_collection(
        "Cd5fG1zS",
        &[TOML, ICON],
        Some(&collection),
        Some(&icon),
        &lookup,
    );
    assert_eq!(messages(&report), ["collection ID is the ID of a project"]);
}

#[test]
fn an_unreadable_file_still_has_its_folder_checked() {
    let report = check_collection("Qm4tZ81c", &[TOML, "extra"], None, None, &lookup);
    assert_eq!(
        messages(&report),
        [
            "unexpected registry entry",
            "expected the collection's PNG icon"
        ]
    );
    assert_eq!(report.publisher, None);
}

#[test]
fn unknown_keys_are_refused() {
    let error =
        ::toml::from_str::<Collection>(&GOOD.replace("name =", "owners = []\nname =")).unwrap_err();
    assert!(error.to_string().contains("unknown field"), "{error}");
}
