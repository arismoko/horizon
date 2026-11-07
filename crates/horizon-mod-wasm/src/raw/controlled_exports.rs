use super::*;
use mod_api::__private::wasm_host::{Budgets, Member, instantiate};
use wasm_encoder::{CodeSection, Function, Instruction as Op, Module, reencode::Reencode};
// Only load metadata and the raw fixture functions get executable bodies.
fn component(names: &[(&str, &str)]) -> Exports {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = root.join(format!("target/raw-exports-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    horizon_mod_api_gen::generate(&root, &dir, horizon_mod_api_gen::Target::Api).unwrap();
    let mut resolve = wit_parser::Resolve::default();
    let package = resolve
        .push_str(
            "api.wit",
            &std::fs::read_to_string(dir.join("mod-api.wit")).unwrap(),
        )
        .unwrap();
    let world = resolve.select_world(&[package], Some("mod")).unwrap();
    let raw_package = resolve.push_str("raw.wit", "package test:raw; interface listeners { enum host-kind { read, server, client, none } declared: func() -> list<tuple<string, string>>; subscribed: func() -> list<tuple<string, string>>; fire: func(place: u32, host: host-kind); } world raw { export listeners; }").unwrap();
    let prefix = horizon_mod_package::manifest::export_prefix("controlled");
    let exports = std::mem::take(&mut resolve.worlds[world].exports);
    for (_, item) in exports {
        let wit_parser::WorldItem::Interface { id, .. } = &item else {
            panic!("interface")
        };
        let name = resolve.interfaces[*id].name.as_ref().unwrap();
        resolve.worlds[world]
            .exports
            .insert(wit_parser::WorldKey::Name(format!("{prefix}-{name}")), item);
    }
    let raw_world = resolve.select_world(&[raw_package], Some("raw")).unwrap();
    let raw_item = resolve.worlds[raw_world].exports[0].clone();
    resolve.worlds[world].exports.insert(
        wit_parser::WorldKey::Name(format!("{prefix}-raw-listeners")),
        raw_item,
    );
    let module =
        wit_component::dummy_module(&resolve, world, wit_parser::ManglingAndAbi::Standard32);
    let mut rewriter = Fixture::new(&module, names);
    let mut patched = Module::new();
    rewriter
        .parse_core_module(&mut patched, wasmparser::Parser::new(0), &module)
        .unwrap();
    let mut data = wasm_encoder::DataSection::new();
    data.active(0, &wasm_encoder::ConstExpr::i32_const(0), rewriter.data);
    patched.section(&data);
    let mut bytes = patched.finish();
    wit_component::embed_component_metadata(
        &mut bytes,
        &resolve,
        world,
        wit_component::StringEncoding::UTF8,
        true,
    )
    .unwrap();
    let bytes = wit_component::ComponentEncoder::default()
        .module(&bytes)
        .unwrap()
        .encode()
        .unwrap();
    // Canonical aliases in this fixture require component interface identities.
    let mut config = wasmtime::Config::new();
    config.consume_fuel(true);
    config.wasm_component_model_implements(true);
    let engine = wasmtime::Engine::new(&config).unwrap();
    let component = wasmtime::component::Component::new(&engine, bytes).unwrap();
    let mut budgets = Budgets::default();
    budgets.fuel = Some(Default::default());
    instantiate(
        &engine,
        &component,
        &[Member {
            id: "controlled",
            exports: prefix,
        }],
        budgets,
        |_| {},
    )
    .unwrap()
    .remove(0)
    .exports
}
struct Fixture {
    functions: std::collections::HashMap<u32, String>,
    next: u32,
    data: Vec<u8>,
    subscribed: i32,
}
impl Fixture {
    fn new(module: &[u8], names: &[(&str, &str)]) -> Self {
        let mut fixture = Self {
            functions: Default::default(),
            next: 0,
            data: vec![0; 16],
            subscribed: 0,
        };
        for payload in wasmparser::Parser::new(0).parse_all(module) {
            match payload.unwrap() {
                wasmparser::Payload::ImportSection(section) => {
                    for import in section.into_imports() {
                        if matches!(import.unwrap().ty, wasmparser::TypeRef::Func(_)) {
                            fixture.next += 1;
                        }
                    }
                }
                wasmparser::Payload::ExportSection(section) => {
                    for export in section {
                        let export = export.unwrap();
                        if export.kind == wasmparser::ExternalKind::Func {
                            fixture.functions.insert(export.index, export.name.into());
                        }
                    }
                }
                _ => {}
            }
        }
        fixture.list(&[("Counter", "internal")]);
        fixture.subscribed = fixture.list(names);
        fixture
    }
    // Canonical ABI list<tuple<string, string>>: a list header, 16-byte
    // elements, and independent UTF-8 byte ranges. Offset zero is an empty list.
    fn list(&mut self, pairs: &[(&str, &str)]) -> i32 {
        let header = self.data.len();
        let entries = header + 8;
        self.data.extend((entries as u32).to_le_bytes());
        self.data.extend((pairs.len() as u32).to_le_bytes());
        self.data.resize(entries + 16 * pairs.len(), 0);
        for (n, (a, b)) in pairs.iter().enumerate() {
            for (field, text) in [a, b].iter().enumerate() {
                let ptr = self.data.len() as u32;
                self.data.extend(text.as_bytes());
                let slot = entries + n * 16 + field * 8;
                self.data[slot..slot + 4].copy_from_slice(&ptr.to_le_bytes());
                self.data[slot + 4..slot + 8].copy_from_slice(&(text.len() as u32).to_le_bytes());
            }
        }
        while !self.data.len().is_multiple_of(4) {
            self.data.push(0);
        }
        header as i32
    }
}
impl Reencode for Fixture {
    type Error = std::convert::Infallible;
    fn memory_type(
        &mut self,
        memory: wasmparser::MemoryType,
    ) -> Result<wasm_encoder::MemoryType, wasm_encoder::reencode::Error<Self::Error>> {
        let mut ty = wasm_encoder::reencode::utils::memory_type(self, memory);
        ty.minimum = 1;
        Ok(ty)
    }
    fn parse_function_body(
        &mut self,
        code: &mut CodeSection,
        body: wasmparser::FunctionBody<'_>,
    ) -> Result<(), wasm_encoder::reencode::Error<Self::Error>> {
        let name = self.functions.get(&self.next).cloned().unwrap_or_default();
        self.next += 1;
        let mut f = Function::new([]);
        if name.contains("raw-listeners") && name.ends_with("subscribed") {
            f.instruction(&Op::I32Const(self.subscribed));
        } else if name.contains("raw-listeners") && name.ends_with("declared") {
            f.instruction(&Op::I32Const(16));
        } else if name.ends_with("subscribed") {
            f.instruction(&Op::I32Const(0));
        } else if name.contains("raw-listeners") && name.ends_with("fire") {
        } else if !name.contains("post-return") {
            return wasm_encoder::reencode::utils::parse_function_body(self, code, body);
        }
        f.instruction(&Op::End);
        code.function(&f);
        Ok(())
    }
}
#[test]
fn manifest_listener_loads_at_its_declared_place() {
    use mod_points::internals::{Op, Refusal, Reply, Type};
    unsafe fn node(_: *mut (), _: bool, _: Op<'_>) -> Result<Reply, Refusal> {
        Err(mod_points::internals::not_here("fixture has no fields"))
    }
    static TABLE: Table = Table {
        types: &[Type {
            path: "Counter",
            hash: "internal",
            node,
            raw_fields: &[],
            raw_gets: &[],
            raw_index: false,
        }],
        places: &[Place {
            place: mod_points::PointId::Tick as usize,
            target: TARGETS[mod_points::PointId::Tick as usize],
            hash: "wire",
            receiver: None,
            receiver_writable: false,
            record: 0,
        }],
        ..Table::EMPTY
    };
    assert!(crate::INTERNALS.set(&TABLE).is_ok());
    let text = format!(
        "id='controlled'\nname='Controlled'\nversion='1.0.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\n[native]\n[[native.internals]]\npath='Counter'\nhash='internal'\n[wasm]\nsha512='{}'\ntoolchain='1.99.1'\n[[inject]]\nid='a'\nat=\"{}\"\nkind='head'\naccess='internals'\nwith='listener'\nhash='signature'\npriority=0\n",
        "0".repeat(128),
        TARGETS[mod_points::PointId::Tick as usize]
    );
    let mut manifest = Manifest::parse(&text).unwrap();
    let exports = component(&[("tick", "wire")]);
    assert!(listen(&manifest, &exports).unwrap().is_some());
    manifest.inject[0].priority = 1;
    assert!(listen(&manifest, &exports).is_err());
}
mod build_script {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../horizon-mod-internals/build.rs"
    ));
    #[test]
    fn listeners_come_from_the_manifest() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let dir = root.join("target/manifest-listener-generation");
        fs::create_dir_all(&dir).unwrap();
        let catalog = r#"[{"path":"horizon_client::game::Game","kind":"struct","signature":"struct Game {}","hash":"g"},{"path":"horizon_client::game::Game<'_>::tick","kind":"method","signature":"fn tick(&mut self)","hash":"h"}]"#;
        fs::write(dir.join("catalog.json"), catalog).unwrap();
        let manifest = format!(
            "id='generation'\nname='Generation'\nversion='1.0.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\n[native]\ninternals=[{{path=\"horizon_client::game::Game<'_>::tick\",hash='h'}}]\n[wasm]\nsha512='{}'\ntoolchain='1.99.1'\n[[inject]]\nid='listener'\nat=\"horizon_client::game::Game<'_>::tick\"\nkind='head'\naccess='internals'\nhash='h'\nwith='chosen'",
            "0".repeat(128)
        );
        // SAFETY: only this test sets the build-script environment in this process.
        unsafe {
            env::set_var(
                "HORIZON_MOD_INTERNALS_MANIFEST",
                dir.join("horizon-mod.toml"),
            );
            env::set_var("HORIZON_MOD_INTERNALS_CATALOG", dir.join("catalog.json"));
        }
        for function in ["chosen", "changed"] {
            fs::write(
                dir.join("horizon-mod.toml"),
                manifest.replace("chosen", function),
            )
            .unwrap();
            let _entry_point = main;
            let code = generate().unwrap();
            assert!(code.contains(&format!("crate :: {function}")));
        }
    }
}
