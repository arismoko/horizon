use super::*;
use crate::{Target, generate as generate_files};
use std::process::Command;

fn put(root: &Path, relative: &str, source: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, source).unwrap();
}

#[test]
#[ignore = "daily"]
fn nominal_packages_compile_and_dispatch_without_a_peer_edge() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let host = "crates/sunrise-mod-vocab/src/host/";
    put(
        root,
        &format!("{host}fixture.rs"),
        r#"
pub trait FixtureHost {
    fn local(&self) -> u32;
    fn shared(&self) -> u32;
    fn operation(&self) -> u32;
    fn common_read(&self) -> u32;
    #[static_call] fn fixed(&self) -> u32;
    #[published(published)] fn fixed_data(&self) -> u32;
}
pub trait FixtureServerHost {
    #[service] fn service(&self) -> &dyn FixtureService;
    fn slot(&self, slot: u32) -> Result<Option<ItemStack>, SlotError>;
}
pub trait FixtureBackendMut {
    fn update(&mut self, value: u32) -> u32;
    fn consume(&mut self, value: ItemStack) -> u32;
}
pub trait FixtureClientBackend { fn desktop(&self) -> Option<u32>; }
"#,
    );
    put(
        root,
        &format!("{host}tables.rs"),
        r#"
#[table_backend("crate::game::BlockTables")]
pub trait TablesBackend { fn table(&self) -> u32; }
"#,
    );
    for dir in ["parts", "exports"] {
        fs::create_dir_all(root.join("crates/sunrise-mod-vocab/src").join(dir)).unwrap();
    }
    put(
        root,
        "crates/sunrise-mod-vocab/src/hooks/fixture.rs",
        r#"
pub trait Hooks {
    /// Shared hook.
    #[hook(side = "both", contract = "listen", dispatch = "flat", receiver = "read", host = "read", target = "sunrise_gameplay_common::game::native_hooks::GameHooks::native_shared")]
    fn shared();
    /// Server hook.
    #[hook(side = "server", contract = "listen", dispatch = "flat", receiver = "write")]
    fn server();
    /// Client hook.
    #[hook(side = "client", contract = "listen", dispatch = "flat", receiver = "read")]
    fn client();
    /// Side wrapper that borrows a portable view.
    #[hook(side = "server", contract = "listen", dispatch = "flat", receiver = "read", host = "read", receiver_type = "sunrise_gameplay_common::game::Borrowed<'_>", target = "sunrise_gameplay::game::native_hooks::fixture::native_borrowed")]
    fn borrowed();
}
"#,
    );
    let mut declarations = String::new();
    for (key, ty) in [("server", "ServerGame"), ("client", "ClientGame")] {
        declarations.push_str(&format!(r#"
[host.{key}]
"FixtureHost::shared" = {{ kind = "shared_trait", owner = "::sunrise_gameplay_common::game::SharedRead", borrow = "shared" }}
"FixtureHost::common_read" = {{ kind = "common", owner = "::sunrise_gameplay_common::game::CommonGame", borrow = "shared" }}
"FixtureHost::operation" = {{ kind = "side_operation", owner = "::sunrise_gameplay_common::game::SideOperation", borrow = "shared" }}
"FixtureHost::fixed" = {{ kind = "static", owner = "::sunrise_gameplay_common::game::CommonGame", borrow = "none" }}
"FixtureHost::local" = {{ kind = "local", owner = "crate::game::{ty}", borrow = "shared" }}

"#));
    }
    declarations.push_str(
        r#"
[items.server]
kind = "shared_trait"
owner = "::sunrise_gameplay_common::game::SharedRead"
borrow = "shared"

[host.server."FixtureBackendMut::update"]
kind = "common"
owner = "::sunrise_gameplay_common::game::CommonGame"
borrow = "mutable"
"#,
    );
    put(root, ROUTING, &declarations);
    let output = root.join("out");
    for (package, target) in [
        (GameplayPackage::Common, Target::GameplayCommon),
        (GameplayPackage::Server, Target::GameplayServer),
        (GameplayPackage::Client, Target::GameplayClient),
    ] {
        let dir = output.join(package.domain().package);
        fs::create_dir_all(&dir).unwrap();
        generate_files(root, &dir, target).unwrap();
        let goal = dir.join(GOAL);
        assert_eq!(goal.is_file(), package == GameplayPackage::Server);
        for name in [FIRE, crate::build::HOST_OUTPUT] {
            let rust = fs::read_to_string(dir.join(name)).unwrap();
            syn::parse_file(&rust).unwrap();
            assert!(!rust.contains("cfg(feature = \"client\")"));
        }
    }

    let compile = |name: &str, src: &str, deps: &[(&str, &Path)], library: bool| {
        let input = if let Some(package) = GameplayPackage::from_library(name) {
            let directory = package.domain().package_path(root);
            let common = if package == GameplayPackage::Common {
                ""
            } else {
                "[dependencies]\nsunrise-gameplay-common = { path = \"../sunrise-gameplay-common\" }\n"
            };
            put(
                root,
                &format!("crates/{}/Cargo.toml", package.domain().package),
                &format!("[package]\nname = {:?}\n{common}", package.domain().package),
            );
            fs::create_dir_all(directory.join("src")).unwrap();
            directory.join("src/lib.rs")
        } else {
            root.join(format!("{name}.rs"))
        };
        fs::write(&input, src).unwrap();
        let file = root.join(if library {
            format!("lib{name}.rlib")
        } else {
            format!("{name}{}", std::env::consts::EXE_SUFFIX)
        });
        let mut rustc = Command::new("rustc");
        rustc
            .args([
                "--edition=2024",
                "--crate-name",
                name,
                "-Dwarnings",
                "--crate-type",
                if library { "rlib" } else { "bin" },
            ])
            .arg("-L")
            .arg(format!("dependency={}", root.display()))
            .arg(&input)
            .arg("-o")
            .arg(&file);
        for (name, path) in deps {
            rustc
                .arg("--extern")
                .arg(format!("{name}={}", path.display()));
        }
        let result = rustc.output().unwrap();
        assert!(
            result.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        file
    };
    let includes = |p: GameplayPackage, name| {
        format!(
            "include!({:?});",
            output.join(p.domain().package).join(name)
        )
    };
    let api = compile(
        "mod_api",
        r#"
pub trait FixtureService {}
pub struct ItemStack(pub u32);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotError { Engine, Conversion }
pub struct BackendRef<'a>(pub &'a dyn Backend);
pub trait Host: prelude::FixtureHost { fn backend(&self) -> Option<BackendRef<'_>> { None } }
pub trait Backend: prelude::TablesBackend + prelude::FixtureClientBackend {}
pub trait ServerHost {}
pub trait ClientHost {}
pub mod prelude {
    pub trait FixtureHost {
        fn local(&self) -> u32;
        fn shared(&self) -> u32;
        fn operation(&self) -> u32;
    fn common_read(&self) -> u32;
        fn fixed(&self) -> u32;
        fn fixed_data(&self) -> u32;
    }
    pub trait FixtureServerHost {
        fn service(&self) -> &dyn crate::FixtureService;
        fn slot(&self, slot: u32) -> Result<Option<crate::ItemStack>, crate::SlotError>;
    }
    pub trait FixtureBackendMut {
        fn update(&mut self, value: u32) -> u32;
        fn consume(&mut self, value: crate::ItemStack) -> u32;
    }
    pub trait FixtureClientBackend { fn desktop(&self) -> Option<u32>; }
    pub trait TablesBackend { fn table(&self) -> u32; }
}
pub mod __private {
    pub mod game {
        pub struct CallbackOwner;
        impl CallbackOwner {
            pub fn call_host<T>(call: impl FnOnce() -> T) -> T { call() }
        }
    }
    pub fn published() -> u32 { 99 }
    pub fn backend_ref(v: &dyn crate::Backend) -> crate::BackendRef<'_> { crate::BackendRef(v) }
    pub fn backend_read<T>(host: &dyn crate::Host, f: impl FnOnce(&dyn crate::Backend) -> T) -> Option<T> {
        host.backend().map(|value| f(value.0))
    }
}
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
pub static CALLS: AtomicU32 = AtomicU32::new(0);
pub fn dispatch_shared(_: &dyn Host) { CALLS.fetch_add(1, Relaxed); }
pub fn dispatch_server(_: &mut dyn ServerHost) { CALLS.fetch_add(10, Relaxed); }
pub fn dispatch_client(_: &dyn ClientHost) { CALLS.fetch_add(100, Relaxed); }
pub fn dispatch_borrowed(_: &dyn Host) { CALLS.fetch_add(1000, Relaxed); }
"#,
        &[],
        true,
    );
    let common = compile(
        "sunrise_gameplay_common",
        &format!(
            r#"
pub mod game {{
    use mod_api::prelude::TablesBackend;
    pub trait SideGame<'g>: SharedRead {{ fn as_host(&self) -> &dyn mod_api::Host; }}
    pub trait SharedRead {{
        fn shared(&self) -> u32 {{ self.items() + 2 }}
        fn items(&self) -> u32;
    }}
    pub trait SideOperation {{ fn operation(&self) -> u32; }}
    pub struct CommonGame<'g>(pub &'g u32);
    impl CommonGame<'_> {{
        pub fn fixed() -> u32 {{ 3 }}
        pub fn common_read(&self) -> u32 {{ *self.0 }}
        pub fn update(&mut self, value: u32) -> u32 {{ *self.0 + value }}
    }}
    pub struct BlockTables<'a>(&'a u32);
    impl<'a> BlockTables<'a> {{
        pub fn of(game: &'a CommonGame<'_>) -> Self {{ Self(game.0) }}
        pub fn table(&self) -> u32 {{ *self.0 }}
    }}
    pub struct Borrowed<'a>(pub &'a u32);
    impl mod_api::Host for Borrowed<'_> {{}}
    impl mod_api::prelude::FixtureHost for Borrowed<'_> {{
        fn local(&self) -> u32 {{ *self.0 }}
        fn shared(&self) -> u32 {{ self.local() }}
        fn operation(&self) -> u32 {{ self.local() }}
        fn common_read(&self) -> u32 {{ self.local() }}
        fn fixed(&self) -> u32 {{ self.local() }}
        fn fixed_data(&self) -> u32 {{ self.local() }}
    }}
    pub mod api_values {{
        pub trait TryIntoEngine {{
            fn try_into_engine(&self, original_items: u32) -> Result<u32, ()>;
        }}
        impl TryIntoEngine for mod_api::ItemStack {{
            fn try_into_engine(&self, original_items: u32) -> Result<u32, ()> {{ Ok(self.0 + original_items) }}
        }}
        pub trait TryIntoApi {{
            type Api;
            fn try_into_api(&self) -> Result<Self::Api, StackReadError>;
        }}
        pub struct StackReadError;
        impl StackReadError {{
            fn into_write_error(self) -> mod_api::SlotError {{ mod_api::SlotError::Conversion }}
        }}
        impl TryIntoApi for Option<u32> {{
            type Api = Option<mod_api::ItemStack>;
            fn try_into_api(&self) -> Result<Self::Api, StackReadError> {{
                match self {{
                    Some(0) => Err(StackReadError),
                    value => Ok(value.map(mod_api::ItemStack)),
                }}
            }}
        }}
        impl<T: TryIntoApi, E: Clone> TryIntoApi for Result<T, E> {{
            type Api = Result<T::Api, E>;
            fn try_into_api(&self) -> Result<Self::Api, StackReadError> {{
                match self {{
                    Ok(value) => value.try_into_api().map(Ok),
                    Err(error) => Ok(Err(error.clone())),
                }}
            }}
        }}
        pub fn call_stack_write<T: TryIntoApi, E: From<mod_api::SlotError>>(
            call: impl FnOnce() -> Result<T, E>,
        ) -> Result<T::Api, E> {{
            call()?.try_into_api().map_err(|error| error.into_write_error().into())
        }}
    }}
    pub mod native_hooks {{
        pub mod fire {{ {} }}
        use fire::FireHooks;
        pub trait GameHooks<'g>: FireHooks<'g> {{
            fn native_shared(&self) {{ <Self as FireHooks<'g>>::fire_shared(self); }}
        }}
        impl<'g, T: FireHooks<'g> + ?Sized> GameHooks<'g> for T {{}}
    }}
    mod host {{ {} }}
    pub fn table(game: &CommonGame<'_>) -> u32 {{ TablesBackend::table(&BlockTables::of(game)) }}
}}
"#,
            includes(GameplayPackage::Common, FIRE),
            includes(GameplayPackage::Common, crate::build::HOST_OUTPUT)
        ),
        &[("mod_api", &api)],
        true,
    );
    let mut sides = Vec::new();
    for (package, ty, name, value) in [
        (GameplayPackage::Server, "ServerGame", "server", 5),
        (GameplayPackage::Client, "ClientGame", "client", 6),
    ] {
        let server = package == GameplayPackage::Server;
        let special = if server {
            r#"
impl mod_api::ServerHost for ServerGame<'_> {}
impl mod_api::FixtureService for ServerGame<'_> {}
impl ServerGame<'_> {
    fn consume(&mut self, value: u32) -> u32 { value }
    fn slot(&self, slot: u32) -> Result<Option<u32>, mod_api::SlotError> {
        match slot {
            1 => Ok(Some(9)),
            2 => Ok(Some(0)),
            _ => Err(mod_api::SlotError::Engine),
        }
    }
}
pub mod goal {
    pub struct GoalCx<'read, 'game> {
        pub host: &'read dyn mod_api::Host,
        pub marker: core::marker::PhantomData<&'game ()>,
    }
    mod generated { GOAL }
}
"#
            .replace("GOAL", &includes(package, GOAL))
        } else {
            "impl mod_api::ClientHost for ClientGame<'_> {} impl ClientGame<'_> { fn desktop(&self) -> Option<u32> { Some(self.client) } }".into()
        };
        let extra = if server {
            r#"
        assert_eq!(FixtureBackendMut::consume(&mut game, mod_api::ItemStack(7)), 12);
        assert_eq!(FixtureBackendMut::update(&mut game, 2), 6);
        let _ = FixtureServerHost::service(&game);
        assert_eq!(FixtureServerHost::slot(&game, 1).unwrap().unwrap().0, 9);
        assert!(matches!(FixtureServerHost::slot(&game, 2), Err(mod_api::SlotError::Conversion)));
        assert!(matches!(FixtureServerHost::slot(&game, 3), Err(mod_api::SlotError::Engine)));
        assert_eq!(FixtureClientBackend::desktop(&game), None);
        native_hooks::fixture::native_borrowed(&common::Borrowed(&4));
            "#
        } else {
            "assert_eq!(FixtureClientBackend::desktop(&game), Some(6));"
        };
        let borrowed = if server {
            "pub fn native_borrowed(view: &sunrise_gameplay_common::game::Borrowed<'_>) { super::fire::fire_borrowed(view); }"
        } else {
            ""
        };
        let mutable = if server { "mut " } else { "" };
        let source = format!(
            r#"
pub mod game {{
    use sunrise_gameplay_common::game as common;
    use common::native_hooks::GameHooks;
    use mod_api::prelude::*;
    pub struct {ty}<'g> {{ pub common: common::CommonGame<'g>, pub {name}: u32 }}
    impl mod_api::Host for {ty}<'_> {{}}
    impl<'g> common::SideGame<'g> for {ty}<'g> {{ fn as_host(&self) -> &dyn mod_api::Host {{ self }} }}
    impl common::SharedRead for {ty}<'_> {{ fn items(&self) -> u32 {{ self.{name} }} }}
    impl common::SideOperation for {ty}<'_> {{ fn operation(&self) -> u32 {{ self.{name} + 1 }} }}
    impl {ty}<'_> {{ fn local(&self) -> u32 {{ self.{name} }} }}
    {special}
    mod host {{ {} }}
    mod native_hooks {{
        mod fire {{ {} }}
        pub mod fixture {{
            use super::super::{ty};
            impl {ty}<'_> {{
                pub fn native_{name}(&{mutable}self) {{ self.fire_{name}(); }}
            }}
            {borrowed}
        }}
    }}
    pub fn run() {{
        let {mutable}game = {ty} {{ common: common::CommonGame(&4), {name}: {value} }};
        game.native_shared();
        game.native_{name}();
        assert_eq!(FixtureHost::local(&game), {value});
        assert_eq!(FixtureHost::shared(&game), {value} + 2);
        assert_eq!(FixtureHost::operation(&game), {value} + 1);
        assert_eq!(FixtureHost::common_read(&game), 4);
        assert_eq!(FixtureHost::fixed(&game), 3);
        assert_eq!(FixtureHost::fixed_data(&game), 99);
        assert_eq!(TablesBackend::table(&game), 4);
        {extra}
    }}
}}
"#,
            includes(package, crate::build::HOST_OUTPUT),
            includes(package, FIRE)
        );
        sides.push(compile(
            package.domain().key,
            &source,
            &[("mod_api", &api), ("sunrise_gameplay_common", &common)],
            true,
        ));
    }
    let runner = compile(
        "desktop",
        r#"
fn main() {
    sunrise_gameplay::game::run();
    sunrise_client_gameplay::game::run();
    assert_eq!(mod_api::CALLS.load(std::sync::atomic::Ordering::Relaxed), 1112);
}
"#,
        &[
            ("mod_api", &api),
            ("sunrise_gameplay", &sides[0]),
            ("sunrise_client_gameplay", &sides[1]),
        ],
        false,
    );
    assert!(Command::new(runner).status().unwrap().success());
    let dedicated = compile(
        "dedicated",
        "fn main() { sunrise_gameplay::game::run(); }",
        &[("sunrise_gameplay", &sides[0])],
        false,
    );
    assert!(Command::new(dedicated).status().unwrap().success());

    let client_source = GameplayPackage::Client
        .domain()
        .package_path(root)
        .join("src/lib.rs");
    let source = fs::read_to_string(&client_source).unwrap();
    let gated = source.replacen(
        "pub mod game {",
        "#[cfg(feature = \"client\")] pub mod game {",
        1,
    );
    assert_ne!(source, gated);
    fs::write(client_source, gated).unwrap();
    put(root, "crates/sunrise-mod-api-gen/source-layout", "split\n");
    let configs = [
        "unix\ntarget_os=\"linux\"".into(),
        "windows\ntarget_os=\"windows\"".into(),
    ];
    crate::check_hook_wiring(root, &configs).unwrap();

    let common_source = GameplayPackage::Common
        .domain()
        .package_path(root)
        .join("src/lib.rs");
    let source = fs::read_to_string(&common_source).unwrap();
    let body = "pub fn common_read(&self) -> u32 { *self.0 }";
    let moved = source.replace(body, "").replace(
        "    pub struct Borrowed",
        r#"    pub trait ShadowRead { fn common_read(&self) -> u32; }
    impl ShadowRead for CommonGame<'_> { fn common_read(&self) -> u32 { *self.0 + 10 } }
    pub fn stale_call(game: &CommonGame<'_>) -> u32 { CommonGame::common_read(game) }
    pub struct Borrowed"#,
    );
    assert_ne!(moved, source);
    compile(
        "sunrise_gameplay_common",
        &moved,
        &[("mod_api", &api)],
        true,
    );
    let error = crate::check_hook_wiring(root, &configs).unwrap_err();
    assert!(
        error.contains("CommonGame::common_read: no matching inherent definition"),
        "{error}"
    );
}
