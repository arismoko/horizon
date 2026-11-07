//! Execute the production conversion and import generators, not a handwritten codec.
use super::*;
use crate::wasm::{Method, glue, index::Index, ty::Types};
use std::{collections::BTreeSet, fs, process::Command};

#[test]
fn generated_host_constructors_refuse_nested_inputs_and_hook_outputs() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    // Read the actual key declarations and their metadata, alongside small shapes that make
    // every nesting position observable without adding a public API just for this test.
    let generated = crate::sources(&root, crate::FillMode::Require).unwrap();
    let included = [
        ("mod_api_generated.rs", generated.hooks.as_str()),
        ("mod_api_generated_host.rs", generated.host.as_str()),
    ];
    let mut types = Types::new(Index::new(&root, &included)).unwrap();
    let module = types.index.module("mod_api", &[]).unwrap();
    let fixture = include_str!("boundary_fixture.rs");
    let declarations = fixture.split("// END DECLARATIONS").next().unwrap();
    let parsed = syn::parse_file(declarations).unwrap();
    types.index.modules[module].items.extend(parsed.items);
    let mut named = |name: &str| {
        types
            .ty(module, &syn::parse_str(name).unwrap(), &[])
            .unwrap()
    };
    let block = named("BlockKey");
    let effect = named("EffectKey");
    let record = named("BoundaryRecord");
    let event = named("BoundaryEvent");
    let Ty::Named(event_id) = event else {
        panic!("event record")
    };
    let methods: Vec<_> = [
        ("block", block.clone()),
        ("effect", effect.clone()),
        ("record", record.clone()),
    ]
    .into_iter()
    .map(|(name, ty)| Method {
        via: None,
        owner: "BoundaryProbe",
        name: name.into(),
        receiver: Some(true),
        params: vec![("input".into(), ty)],
        ret: Ty::Prim("u32", "u32"),
    })
    .collect();
    let event_method = Method {
        via: None,
        owner: "BoundaryProbe",
        name: "event".into(),
        receiver: Some(true),
        params: vec![("event".into(), Ty::Ref(Box::new(event), true))],
        ret: Ty::Unit,
    };
    wit::refuse(&mut types);
    let (defs, _) = wit::reached(
        &types,
        &methods.iter().chain([&event_method]).collect::<Vec<_>>(),
    );
    let host = glue::conversions(&types, &defs, &BTreeSet::from([event_id]));
    let imports = methods
        .iter()
        .map(|m| host_import(&types, m, "self", false).unwrap())
        .collect::<String>();
    let guest_block = {
        types.guest.set(true);
        let code = glue::conversions(&types, &defs, &BTreeSet::new());
        types.guest.set(false);
        code
    };
    let (fixture, tests) = fixture.split_once("// TESTS").unwrap();
    let mut source = fixture.replace("// END DECLARATIONS", "");
    // The fixture supplies wire data only; every lifting/lowering/write-back expression and
    // imported method body below comes from the production generator.
    source.push_str(&format!(r#"
mod host {{
    use super::*;
    {host}
    impl Probe {{ {imports} }}
    pub fn write(event: &mut BoundaryEvent, wire: horizon::mod_api::types::BoundaryEventOut) -> Result<(), Refused> {{ write_back_{event_id}(event, wire) }}
    {tests}
    pub fn lower_block(key: &BlockKey) -> Result<u32, Refused> {{ Ok({lower_block}) }}
}}
mod guest {{
    use super::*;
    {guest_block}
    pub fn block(slot: u32) -> Result<BlockKey, Refused> {{ Ok({guest_lift}) }}
}}
"#, lower_block = glue::lower(&types, &block, "key"), guest_lift = glue::lift(&types, &block, "slot")));
    // Build output remains in the ignored checkout target, never /tmp.
    let dir = root
        .join("target")
        .join(format!("key-boundary-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let src = dir.join("boundary.rs");
    let binary = dir.join("boundary-tests");
    fs::write(&src, source).unwrap();
    let built = Command::new("rustc")
        .args(["--edition=2024", "--test", "-Awarnings"])
        .arg(&src)
        .arg("-o")
        .arg(&binary)
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let ran = Command::new(&binary)
        .arg("--nocapture")
        .current_dir(&dir)
        .output()
        .unwrap();
    let output = String::from_utf8_lossy(&ran.stdout);
    assert!(
        ran.status.success(),
        "{output}\n{}",
        String::from_utf8_lossy(&ran.stderr)
    );
    assert!(output.contains("4 passed"), "{output}");
    println!("{output}");
    fs::remove_dir_all(dir).unwrap();
}
