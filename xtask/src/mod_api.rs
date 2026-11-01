//! Generate the typed mod interface from native hook signatures and catalog contracts.
use quote::ToTokens;
use std::{collections::HashSet, fs, path::Path, process::Command};
use syn::{FnArg, ImplItem, Item, Pat, Type, visit::Visit};

fn has_dispatch(method: &syn::ImplItemFn, name: &str) -> bool {
    struct Calls<'a> {
        expected: &'a str,
        found: bool,
    }
    impl<'ast> Visit<'ast> for Calls<'_> {
        fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
            if let syn::Expr::Path(path) = &*call.func
                && path
                    .path
                    .segments
                    .last()
                    .is_some_and(|s| s.ident == self.expected)
            {
                self.found = true;
            }
            syn::visit::visit_expr_call(self, call);
        }
    }
    let expected = format!("dispatch_{name}");
    let mut calls = Calls {
        expected: &expected,
        found: false,
    };
    calls.visit_block(&method.block);
    calls.found
}

fn fresh_name(base: &str, used: &mut HashSet<String>) -> String {
    let mut name = base.to_owned();
    while !used.insert(name.clone()) {
        name.push('_');
    }
    name
}

fn host_for(target: &str, mutable: Option<bool>) -> Option<(&str, String)> {
    mutable.map(|mutable| {
        let (name, kind) = if target.contains("::GameLevel<") {
            ("level", "LevelHost")
        } else {
            ("host", "Host")
        };
        let mutable = if mutable { "mut " } else { "" };
        (name, format!("&{mutable}dyn {kind}"))
    })
}

pub fn run(check: bool) -> Result<(), String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let catalog: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(root.join("native-hooks.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let source = fs::read_to_string(root.join("crates/horizon-client/src/game/native_hooks.rs"))
        .map_err(|e| e.to_string())?;
    let file = syn::parse_file(&source).map_err(|e| e.to_string())?;
    let mut methods = Vec::new();
    for item in file.items {
        if let Item::Impl(item) = item {
            for member in item.items {
                if let ImplItem::Fn(method) = member {
                    methods.push(method);
                }
            }
        }
    }
    let mut traits = String::new();
    let mut dispatch = String::new();
    let mut seen = HashSet::new();
    for hook in catalog["hooks"]
        .as_array()
        .ok_or("catalog hooks must be an array")?
    {
        let Some(api) = hook.get("mod_api") else {
            continue;
        };
        let name = hook["name"].as_str().ok_or("hook name missing")?;
        if !seen.insert(name.to_owned()) {
            return Err(format!("duplicate hook {name}"));
        }
        let native = format!("native_{name}");
        let matches: Vec<_> = methods.iter().filter(|m| m.sig.ident == native).collect();
        let [method] = matches.as_slice() else {
            return Err(format!("expected one {native} signature"));
        };
        if !hook["target"]
            .as_str()
            .is_some_and(|target| target.ends_with(&format!("::{native}")))
        {
            return Err(format!("wrong target for {name}"));
        }
        if !has_dispatch(method, name) {
            return Err(format!("missing dispatch_{name} call in {native}"));
        }
        let sig = &method.sig;
        let mut args = Vec::new();
        let mut receiver_mut = None;
        for arg in &sig.inputs {
            match arg {
                FnArg::Receiver(r) => {
                    receiver_mut = Some(r.mutability.is_some());
                }
                FnArg::Typed(arg) => {
                    let Pat::Ident(pat) = &*arg.pat else {
                        return Err(format!("unsupported argument in {name}"));
                    };
                    args.push((pat.ident.to_string(), &*arg.ty));
                }
            }
        }
        let mut used: HashSet<_> = args.iter().map(|(name, _)| name.clone()).collect();
        let mod_list = fresh_name("mods", &mut used);
        let mod_item = fresh_name("mod_hook", &mut used);
        let replacement = fresh_name("replacement", &mut used);
        let contract = api["contract"].as_str().ok_or("contract missing")?;
        let out = api.get("out").and_then(|v| v.as_str());
        if !matches!(
            (contract, out),
            ("cancel" | "replace" | "collect", Some(_)) | ("listen", None)
        ) {
            return Err(format!("invalid contract for {name}"));
        }
        let output = if let Some(out) = out {
            let index = args
                .iter()
                .position(|(n, _)| n == out)
                .ok_or_else(|| format!("missing output {out} in {name}"))?;
            Some(args.remove(index))
        } else {
            None
        };
        let result = if let Some((_, ty)) = &output {
            let Type::Reference(reference) = ty else {
                return Err(format!("output must be a mutable reference in {name}"));
            };
            if reference.mutability.is_none() {
                return Err(format!("output must be a mutable reference in {name}"));
            }
            match contract {
                "cancel" if reference.elem.to_token_stream().to_string() == "bool" => {
                    " -> Flow".to_owned()
                }
                "replace" => format!(" -> {}", reference.elem.to_token_stream()),
                "collect" => String::new(),
                _ => return Err(format!("unexpected output type for {name}")),
            }
        } else {
            String::new()
        };
        let host = host_for(
            hook["target"].as_str().ok_or("target missing")?,
            receiver_mut,
        )
        .map(|(name, ty)| (fresh_name(name, &mut used), ty));
        let mut trait_params = vec!["&self".to_owned()];
        let mut dispatch_params = Vec::new();
        let mut call_args = Vec::new();
        let mut unused = String::new();
        if let Some((name, ty)) = &host {
            trait_params.push(format!("{name}: {ty}"));
            dispatch_params.push(format!("{name}: {ty}"));
            call_args.push(name.clone());
            unused.push_str(&format!("let _ = {name};"));
        }
        dispatch_params.push(format!("{mod_list}: &[Box<dyn Hooks>]"));
        for (name, ty) in &args {
            let param = format!("{name}: {}", ty.to_token_stream());
            trait_params.push(param.clone());
            dispatch_params.push(param);
            call_args.push(match ty {
                Type::Reference(r) if r.mutability.is_some() => format!("&mut *{name}"),
                _ => name.clone(),
            });
            unused.push_str(&format!("let _ = {name};"));
        }
        if let Some((name, ty)) = &output {
            let param = format!("{name}: {}", ty.to_token_stream());
            dispatch_params.push(param.clone());
            if contract == "collect" {
                trait_params.push(param);
                call_args.push(format!("&mut *{name}"));
                unused.push_str(&format!("let _ = {name};"));
            }
        }
        let default = match contract {
            "cancel" => "Flow::Continue",
            "replace" => "None",
            _ => "",
        };
        let invoke = call_args.join(", ");
        let trait_params = trait_params.join(", ");
        let dispatch_params = dispatch_params.join(", ");
        for attr in &method.attrs {
            if attr.path().is_ident("doc") {
                let syn::Meta::NameValue(value) = &attr.meta else {
                    return Err(format!("unsupported doc attribute in {name}"));
                };
                let syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(doc),
                    ..
                }) = &value.value
                else {
                    return Err(format!("unsupported doc attribute in {name}"));
                };
                for line in doc.value().lines() {
                    traits.push_str(&format!("///{line}\n"));
                }
            }
        }
        match (contract, out) {
            ("cancel", Some(out)) => traits.push_str(&format!(
                "///\n/// Return `Flow::Cancel` to take it: this sets `{out}`, and later mods are skipped.\n"
            )),
            ("replace", Some(out)) => traits.push_str(&format!(
                "///\n/// Return `Some` to set `{out}`; the first mod returning `Some` wins.\n"
            )),
            ("collect", Some(out)) => traits.push_str(&format!(
                "///\n/// Every mod gets `{out}` in order and may add, change or remove {out}.\n"
            )),
            _ => {}
        }
        traits.push_str(&format!(
            "fn {name}({trait_params}){result} {{ {unused} {default} }}\n"
        ));
        let body = match contract {
            "cancel" => format!(
                "if *{o} {{ return; }} for {mod_item} in {mod_list} {{ if matches!({mod_item}.{name}({invoke}), Flow::Cancel) {{ *{o} = true; break; }} }}",
                o = out.unwrap()
            ),
            "replace" => format!(
                "if {o}.is_some() {{ return; }} for {mod_item} in {mod_list} {{ if let Some({replacement}) = {mod_item}.{name}({invoke}) {{ *{o} = Some({replacement}); break; }} }}",
                o = out.unwrap()
            ),
            _ => format!("for {mod_item} in {mod_list} {{ {mod_item}.{name}({invoke}); }}"),
        };
        dispatch.push_str(&format!(
            "pub fn dispatch_{name}({dispatch_params}) {{ {body} }}\n"
        ));
    }
    if seen.is_empty() {
        return Err("no mod API contracts".into());
    }
    let code = format!(
        "// @generated by cargo run -p xtask -- mod-api; do not edit.\nuse super::*;\npub trait Hooks: Send + Sync {{\n{traits}}}\n{dispatch}"
    );
    // Parse before formatting, so invalid catalog names or types fail without writing output.
    syn::parse_file(&code).map_err(|e| format!("generated Rust: {e}"))?;
    let mut child = Command::new("rustfmt")
        .args(["--emit", "stdout", "--edition", "2021"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    use std::io::Write;
    child
        .stdin
        .take()
        .ok_or("rustfmt stdin")?
        .write_all(code.as_bytes())
        .map_err(|e| e.to_string())?;
    let formatted = child.wait_with_output().map_err(|e| e.to_string())?;
    if !formatted.status.success() {
        return Err("rustfmt failed".into());
    }
    let path = root.join("crates/horizon-client/src/mod_api/generated.rs");
    if check {
        if fs::read(&path).map_err(|e| e.to_string())? != formatted.stdout {
            return Err("mod API output is stale; run cargo run -p xtask -- mod-api".into());
        }
    } else {
        fs::create_dir_all(path.parent().ok_or("output parent missing")?)
            .map_err(|e| e.to_string())?;
        fs::write(path, formatted.stdout).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    struct ItemStack;
    #[allow(dead_code)]
    mod stubs {
        #[derive(Clone, Copy)]
        pub(super) struct BlockStateId;
        pub(super) struct Compound;
        #[derive(Clone, Copy)]
        pub(super) struct Direction;
        pub(super) type NativeDispensed = (Option<super::ItemStack>, i32);
        pub(super) struct NativeLootDrops<'a>(pub &'a ());
        pub(super) struct NativeScreenView<'a>(pub &'a ());
        pub(super) struct Corners;
        pub(super) struct HudBatch;
        pub(super) struct NativeBlockRim;
        pub(super) struct NativeColoredLight;
        pub(super) struct NativeMinimap;
        pub(super) struct NativeMod;
        pub(super) struct NativePanelEvent;
        pub(super) struct NativeScreen;
        pub(super) struct ModBlockEntity;
        pub(super) struct ModContainer;
        pub(super) struct ModFluidTanks;
        pub(super) struct NativeCommand;
        pub(super) struct NativeLifecycleContext;
        pub(super) struct NativeServerPlayer;
        #[derive(Clone, Copy)]
        pub(super) struct ServerTickPhase;
        #[derive(Clone, Copy)]
        pub(super) struct WorldPhase;
    }
    use stubs::*;
    struct Text;
    #[derive(Debug, PartialEq)]
    struct NativeZoom(u32);
    enum Flow {
        Continue,
        Cancel,
    }
    trait Host {}
    struct TestHost;
    impl Host for TestHost {}
    #[allow(dead_code)]
    mod generated {
        mod horizon_client {
            pub mod hud_layout {
                pub struct HealthInputs;
                pub struct GuiDraw;
            }
        }
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../crates/horizon-client/src/mod_api/generated.rs"
        ));
    }
    use generated::*;
    struct Mod {
        id: u32,
        calls: Arc<Mutex<Vec<u32>>>,
    }
    impl Hooks for Mod {
        fn level_tick_pre(&self, _: &mut dyn Host, _: &str) {
            self.calls.lock().unwrap().push(self.id);
        }
        fn tooltip(&self, _: &dyn Host, _: &ItemStack, lines: &mut Vec<(Text, u32)>) {
            lines.push((Text, self.id));
        }
        fn zoom(&self, _: &dyn Host) -> Option<NativeZoom> {
            self.calls.lock().unwrap().push(self.id);
            Some(NativeZoom(self.id))
        }
        fn should_apply_effect_tick_this_tick(
            &self,
            _: &dyn Host,
            _: &str,
            _: i32,
            _: i32,
            apply: &mut bool,
        ) {
            *apply = !*apply;
            self.calls.lock().unwrap().push(self.id);
        }
        fn living_incoming_damage(
            &self,
            _: i32,
            _: &str,
            _: &str,
            _: i32,
            _: i32,
            amount: &mut f32,
        ) -> Flow {
            *amount += self.id as f32;
            if self.id == 2 {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
        fn use_item_on(&self, _: &mut dyn Host, _: &ItemStack, _: [i32; 3], _: [i32; 3]) -> Flow {
            self.calls.lock().unwrap().push(self.id);
            if self.id == 1 {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
    }
    #[test]
    fn generated_locals_do_not_shadow_hook_arguments() {
        let mut used = ["mods", "mods_", "mod_hook", "replacement", "host"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        assert_eq!(super::fresh_name("mods", &mut used), "mods__");
        assert_eq!(super::fresh_name("mod_hook", &mut used), "mod_hook_");
        assert_eq!(super::fresh_name("replacement", &mut used), "replacement_");
        assert_eq!(super::fresh_name("host", &mut used), "host_");
    }

    #[test]
    fn level_hosts_match_receiver_mutability() {
        let target = "horizon_client::game::native_hooks::GameLevel<'_>::native_block_changed";
        assert_eq!(
            super::host_for(target, Some(false)),
            Some(("level", "&dyn LevelHost".into()))
        );
        assert_eq!(
            super::host_for(target, Some(true)),
            Some(("level", "&mut dyn LevelHost".into()))
        );
        assert_eq!(super::host_for(target, None), None);
        assert_eq!(
            super::host_for("Game::native_tick", Some(true)),
            Some(("host", "&mut dyn Host".into()))
        );
    }

    #[test]
    fn generated_output_is_current() {
        super::run(true).unwrap();
    }

    #[test]
    fn contracted_hook_requires_dispatch_call() {
        let missing: syn::ImplItemFn = syn::parse_quote! { fn native_zoom(&self) { unrelated(); } };
        let present: syn::ImplItemFn = syn::parse_quote! { fn native_zoom(&self) { horizon_client::mod_api::dispatch_zoom(); } };
        assert!(!super::has_dispatch(&missing, "zoom"));
        assert!(super::has_dispatch(&present, "zoom"));
    }

    #[test]
    fn dispatch_contracts() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mods: Vec<Box<dyn Hooks>> = (1..=2)
            .map(|id| {
                Box::new(Mod {
                    id,
                    calls: calls.clone(),
                }) as Box<dyn Hooks>
            })
            .collect();
        let mut host = TestHost;
        dispatch_level_tick_pre(&mut host, &mods, "overworld");
        assert_eq!(*calls.lock().unwrap(), [1, 2]);
        calls.lock().unwrap().clear();
        let mut zoom = None;
        dispatch_zoom(&host, &mods, &mut zoom);
        assert_eq!(zoom, Some(NativeZoom(1)));
        assert_eq!(*calls.lock().unwrap(), [1]);
        calls.lock().unwrap().clear();
        dispatch_zoom(&host, &mods, &mut zoom);
        assert!(calls.lock().unwrap().is_empty());
        let mut lines = Vec::new();
        dispatch_tooltip(&host, &mods, &ItemStack, &mut lines);
        assert_eq!(lines.iter().map(|line| line.1).collect::<Vec<_>>(), [1, 2]);
        let mut apply = false;
        calls.lock().unwrap().clear();
        dispatch_should_apply_effect_tick_this_tick(&host, &mods, "effect", 0, 0, &mut apply);
        assert!(!apply);
        assert_eq!(*calls.lock().unwrap(), [1, 2]);
        let mut amount = 1.0;
        let mut canceled = false;
        dispatch_living_incoming_damage(
            &mods,
            0,
            "kind",
            "damage",
            0,
            0,
            &mut amount,
            &mut canceled,
        );
        assert_eq!(amount, 4.0);
        assert!(canceled);
        calls.lock().unwrap().clear();
        let mut claimed = false;
        dispatch_use_item_on(&mut host, &mods, &ItemStack, [0; 3], [0; 3], &mut claimed);
        assert!(claimed);
        assert_eq!(*calls.lock().unwrap(), [1]);
        calls.lock().unwrap().clear();
        dispatch_use_item_on(&mut host, &mods, &ItemStack, [0; 3], [0; 3], &mut claimed);
        assert!(calls.lock().unwrap().is_empty());
    }
}
