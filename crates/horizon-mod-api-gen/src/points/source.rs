//! Private source-build point rows. These are never part of the published point catalog.
use super::*;

/// Launcher-owned staging input and the resulting private row metadata.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub name: String,
    pub target: String,
    pub items: String,
    pub module: String,
    /// The stable facade export, not the physical private record declared by `items`.
    pub record_export: String,
}

/// The private plan in a disposable staged tree, not a mod manifest.
pub const FILE: &str = "source-points.toml";

#[derive(serde::Serialize, serde::Deserialize)]
struct Plan {
    points: Vec<Point>,
}

/// Read only the launcher-owned source-build input; an ordinary game has no private rows.
pub fn read(root: &Path) -> Result<Vec<Point>, String> {
    let path = root.join(FILE);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(path).map_err(|error| error.to_string())?;
    toml::from_str::<Plan>(&text)
        .map(|plan| plan.points)
        .map_err(|error| error.to_string())
}

/// Track an existing private build input; tracking an absent path would invalidate every build.
pub fn build_input(root: &Path) -> Result<Vec<Point>, String> {
    let path = root.join(FILE);
    if path.exists() {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    read(root)
}

pub(crate) fn eligible(attrs: &[syn::Attribute], sig: &syn::Signature) -> bool {
    point_shape::eligible(
        &attrs
            .iter()
            .filter(|attr| !attr.path().is_ident("inline"))
            .cloned()
            .collect::<Vec<_>>(),
        sig,
    )
}

/// Mark checked source targets after normal point staging, retaining their lexical type scope.
pub fn stage(root: &Path, points: &mut [Point]) -> Result<(), String> {
    let below = below_points(root)?;
    let marked = |function: &Found<'_>| {
        function.attrs.iter().any(|attr| {
            attr.path()
                .segments
                .last()
                .is_some_and(|segment| segment.ident == "point")
        })
    };
    let (scanned, list) = scan(root)?;
    let mut names = std::collections::BTreeSet::new();
    for (_, function) in all_points(&scanned, &list)? {
        if !marked(&function) {
            return Err(format!(
                "E_SOURCE_ORDER: {}: stage ordinary points first",
                function.target()
            ));
        }
        let shape = function.shape()?;
        names.extend([
            shape.variant().to_string(),
            shape.return_variant().to_string(),
        ]);
    }
    for point in points.iter() {
        for name in [
            point_shape::camel(&point.name),
            format!("{}Return", point_shape::camel(&point.name)),
        ] {
            if !names.insert(name.clone()) {
                return Err(format!("E_SOURCE_COLLISION: {name} for {}", point.target));
            }
        }
    }
    let mut packages = std::collections::BTreeSet::new();
    for point in points.iter_mut() {
        let krate = point
            .target
            .split("::")
            .next()
            .expect("checked source path");
        if below.contains(krate) {
            return Err(format!(
                "E_SOURCE_DEPENDENCY: {} is below mod_points; adding its dispatch would create a Cargo cycle",
                point.target
            ));
        }
        let files = if krate == "horizon_client" {
            game_files(&root.join("crates/horizon-client/src"), true)?
        } else {
            packages.insert(crate_root(root, krate)?.0);
            crate_files(root, krate)?
        };
        let mut matched = false;
        for file in files {
            let text = fs::read_to_string(&file.path).map_err(|error| error.to_string())?;
            let parsed = syn::parse_file(&text).map_err(|error| error.to_string())?;
            let mut found = Vec::new();
            file_functions(
                &file.module,
                &parsed.items,
                &|candidate| plain_path(&candidate.target()) == plain_path(&point.target),
                &eligible,
                &mut found,
            );
            for function in found {
                if matched {
                    return Err(format!("E_INJECT_LOST: {} is ambiguous", point.target));
                }
                if marked(&function) {
                    return Err(format!(
                        "{}: source carries a hand-written #[point]",
                        point.target
                    ));
                }
                let start = match function.vis {
                    syn::Visibility::Inherited => function.sig.fn_token.span.start(),
                    vis => syn::spanned::Spanned::span(vis).start(),
                };
                let lines = std::iter::once(0)
                    .chain(text.match_indices('\n').map(|(i, _)| i + 1))
                    .collect::<Vec<_>>();
                let at = offset(&text, &lines, start);
                let end = offset(&text, &lines, function.end);
                let free = if function.owner.is_none() {
                    ", free"
                } else {
                    ""
                };
                let attribute = format!(
                    "#[horizon_mod_api_macros::point(raw_only, name = {:?}{free})] ",
                    point.name
                );
                let mut out = text.clone();
                out.insert_str(end, &format!("\n{}\n", point.items));
                out.insert_str(at, &attribute);
                // Attribute spans precede both insertions; remove backwards to retain their offsets.
                for attr in function.attrs.iter().rev().filter(|attr| {
                    attr.path().is_ident("inline")
                        && attr
                            .parse_args::<syn::Ident>()
                            .is_ok_and(|arg| arg == "always")
                }) {
                    let span = syn::spanned::Spanned::span(attr);
                    out.replace_range(
                        offset(&text, &lines, span.start())..offset(&text, &lines, span.end()),
                        "",
                    );
                }
                fs::write(&file.path, out).map_err(|error| error.to_string())?;
                point.module = function.module.clone();
                point.record_export = format!("__{}Raw", point_shape::camel(&function.name()));
                matched = true;
            }
        }
        if !matched {
            return Err(format!("E_INJECT_MISSING: {}", point.target));
        }
    }
    dependencies(root, packages)?;
    let plan = Plan {
        points: points.to_vec(),
    };
    fs::write(
        root.join(FILE),
        toml::to_string(&plan).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())
}

#[test]
fn private_build_dependencies_track_only_existing_plans() {
    const ENV: &str = "HORIZON_PRIVATE_BUILD_INPUT_TEST_ROOT";
    if let Some(root) = std::env::var_os(ENV) {
        assert!(build_input(Path::new(&root)).unwrap().is_empty());
        return;
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../target/private-build-input-{}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    for exists in [false, true] {
        if exists {
            fs::write(root.join(FILE), "points = []").unwrap();
        }
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "points::source::private_build_dependencies_track_only_existing_plans",
                "--nocapture",
            ])
            .env(ENV, &root)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).contains("cargo:rerun-if-changed="),
            exists
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "manual real-game public emitter snapshot"]
fn public_points_snapshot() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = std::path::PathBuf::from(std::env::var_os("HORIZON_POINT_SNAPSHOT").unwrap());
    fs::create_dir_all(&out).unwrap();
    crate::generate_points(&root, &out).unwrap();
}

#[test]
fn private_rows_do_not_become_listed_and_handwritten_points_stay_refused() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../target/private-source-{}",
        std::process::id()
    ));
    let put = |path: &str, text: &str| {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    };
    const GAME: &str = "crates/horizon-client/src/game.rs";
    fs::create_dir_all(root.join("crates/horizon-mod-api-gen/slots")).unwrap();
    put(
        "crates/horizon-mod-points/Cargo.toml",
        "[package]\nname='horizon-mod-points'\n",
    );
    put(
        GAME,
        "pub struct Game; impl Game { fn listed(&mut self) {} } mod nested { use super::Game; impl Game { #[must_use] #[inline(always)] fn hidden(&mut self, value: i32) -> i32 { value } #[inline(always)] fn untouched(&self) {} } }",
    );
    let sig = syn::parse_quote!(fn hidden(&mut self, value: i32) -> i32);
    let shape = point_shape::shape(quote!(raw, name = "source123"), &sig).unwrap();
    let mut private = vec![Point {
        name: "source123".into(),
        target: "horizon_client::game::nested::Game::hidden".into(),
        items: shape
            .raw_items(&sig, Some(&syn::parse_quote!(Game)), None)
            .to_string(),
        module: String::new(),
        record_export: String::new(),
    }];
    assert!(
        stage(&root, &mut private)
            .unwrap_err()
            .contains("E_SOURCE_ORDER")
    );
    super::stage(&root).unwrap();
    let before = serde_json::to_value(game_points(&root).unwrap()).unwrap();
    let staged = fs::read_to_string(root.join(GAME)).unwrap();
    put(
        GAME,
        &staged.replace(
            "#[inline(always)]",
            "#[horizon_mod_api_macros::point(raw_only)] #[inline(always)]",
        ),
    );
    assert!(
        stage(&root, &mut private)
            .unwrap_err()
            .contains("source carries a hand-written")
    );
    put(GAME, &staged);
    let mut collision = private.clone();
    collision[0].name = "listed".into();
    assert!(
        stage(&root, &mut collision)
            .unwrap_err()
            .contains("E_SOURCE_COLLISION: Listed")
    );
    let mut cycle = private.clone();
    cycle[0].target = "horizon_mod_points::blocked".into();
    assert!(
        stage(&root, &mut cycle)
            .unwrap_err()
            .contains("E_SOURCE_DEPENDENCY: horizon_mod_points::blocked")
    );
    let mut expected = private[0].clone();
    expected.module = "horizon_client::game::nested".into();
    expected.record_export = "__HiddenRaw".into();
    stage(&root, &mut private).unwrap();
    assert_eq!(private, [expected.clone()]);
    assert_eq!(read(&root).unwrap(), [expected]);
    let staged = syn::parse_file(&fs::read_to_string(root.join(GAME)).unwrap()).unwrap();
    let mut found = Vec::new();
    file_functions(
        "horizon_client::game",
        &staged.items,
        &|function| function.target() == private[0].target,
        &|_, _| true,
        &mut found,
    );
    assert_eq!(found.len(), 1);
    assert!(point_shape::eligible(found[0].attrs, found[0].sig));
    let marker = found[0]
        .attrs
        .iter()
        .find(|attr| attr.path().segments.last().unwrap().ident == "point")
        .unwrap();
    let syn::Meta::List(marker) = &marker.meta else {
        panic!("point arguments")
    };
    assert!(
        point_shape::shape(marker.tokens.clone(), found[0].sig)
            .unwrap()
            .raw_only
    );
    assert!(
        found[0]
            .attrs
            .iter()
            .any(|attr| attr.path().is_ident("must_use"))
    );
    let mut untouched = Vec::new();
    file_functions(
        "horizon_client::game",
        &staged.items,
        &|function| function.sig.ident == "untouched",
        &|_, _| true,
        &mut untouched,
    );
    assert_eq!(untouched.len(), 1);
    assert!(!point_shape::eligible(untouched[0].attrs, untouched[0].sig));
    let public = game_points(&root).unwrap();
    assert_eq!(before, serde_json::to_value(&public).unwrap());
    let mut spaced = private.clone();
    spaced[0].target = "horizon_client :: game :: nested :: Game :: hidden".into();
    fs::write(
        root.join(FILE),
        toml::to_string(&Plan { points: spaced }).unwrap(),
    )
    .unwrap();
    assert_eq!(
        before,
        serde_json::to_value(game_points(&root).unwrap()).unwrap()
    );
    let parsed = syn::parse_file(&records(&public, &private)).unwrap();
    // Compile the actual enum and wire tables together, without unrelated API dispatch bodies.
    let ids = parsed
        .items
        .iter()
        .filter(|item| match item {
            Item::Enum(item) => item.ident == "PointId",
            Item::Const(item) => item.ident == "POINT_COUNT",
            Item::Mod(item) => item.ident == "places",
            _ => false,
        })
        .map(ToTokens::to_token_stream)
        .collect::<proc_macro2::TokenStream>();
    let file = root.join("ids.rs");
    fs::write(
        &file,
        format!(
            r#"
pub struct Game;
mod nested {{ use super::Game; {} }}
pub use nested::__Source123Raw as {};
{ids}
fn main() {{
    assert_eq!(POINT_COUNT, 4);
    assert_eq!(PointId::Listed as usize, 0);
    assert_eq!(PointId::Source123 as usize, 2);
    assert_eq!(places::NAMES, ["listed", "listed_return", "source123", "source123_return"]);
    assert_eq!(&places::SHAPES[2..], ["", ""]);
    assert_eq!(&places::TARGETS[2..], ["horizon_client::game::nested::Game::hidden"; 2]);
    for (index, id) in places::IDS.iter().enumerate() {{ assert_eq!(*id as usize, index); }}
}}
"#,
            private[0].items, private[0].record_export
        ),
    )
    .unwrap();
    let binary = root.join(format!("ids{}", std::env::consts::EXE_SUFFIX));
    let output = std::process::Command::new("rustc")
        .args(["--edition=2024", "--cfg", "feature=\"wasm-host\""])
        .arg(&file)
        .arg("-o")
        .arg(&binary)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        std::process::Command::new(&binary)
            .status()
            .unwrap()
            .success()
    );
    assert!(super::stage(&root).unwrap_err().contains("already has"));
    fs::remove_dir_all(root).unwrap();
}
