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
        let mut receiver_mut = false;
        for arg in &sig.inputs {
            match arg {
                FnArg::Receiver(r) => receiver_mut = r.mutability.is_some(),
                FnArg::Typed(arg) => {
                    let Pat::Ident(pat) = &*arg.pat else {
                        return Err(format!("unsupported argument in {name}"));
                    };
                    args.push((pat.ident.to_string(), &*arg.ty));
                }
            }
        }
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
        let host = if receiver_mut {
            "&mut dyn Host"
        } else {
            "&dyn Host"
        };
        let inputs = args
            .iter()
            .map(|(name, ty)| format!(", {name}: {}", ty.to_token_stream()))
            .collect::<String>();
        let call = args
            .iter()
            .map(|(name, _)| format!(", {name}"))
            .collect::<String>();
        let default = match contract {
            "cancel" => "Flow::Continue",
            "replace" => "None",
            _ => "",
        };
        let output_param = output
            .as_ref()
            .map(|(name, ty)| format!(", {name}: {}", ty.to_token_stream()))
            .unwrap_or_default();
        let (trait_extra, call_extra) = if contract == "collect" {
            (output_param.as_str(), format!(", {}", out.unwrap()))
        } else {
            ("", String::new())
        };
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
        let unused = if call.is_empty() && call_extra.is_empty() {
            "host".to_owned()
        } else {
            format!("(host{call}{call_extra},)")
        };
        traits.push_str(&format!("fn {name}(&self, host: {host}{inputs}{trait_extra}){result} {{ let _ = {unused}; {default} }}\n"));
        let body = match contract {
            "cancel" => format!(
                "if *{o} {{ return; }} for m in mods {{ if matches!(m.{name}(host{call}), Flow::Cancel) {{ *{o} = true; break; }} }}",
                o = out.unwrap()
            ),
            "replace" => format!(
                "if {o}.is_some() {{ return; }} for m in mods {{ if let Some(value) = m.{name}(host{call}) {{ *{o} = Some(value); break; }} }}",
                o = out.unwrap()
            ),
            "collect" => format!("for m in mods {{ m.{name}(host{call}{call_extra}); }}"),
            _ => format!("for m in mods {{ m.{name}(host{call}); }}"),
        };
        dispatch.push_str(&format!("pub fn dispatch_{name}(host: {host}, mods: &[Box<dyn Hooks>]{inputs}{output_param}) {{ {body} }}\n"));
    }
    if seen.is_empty() {
        return Err("no mod API contracts".into());
    }
    let code = format!(
        "// @generated by cargo run -p xtask -- mod-api; do not edit.\nuse super::{{Flow, Host, ItemStack, NativeZoom, Text}};\npub trait Hooks: Send + Sync {{\n{traits}}}\n{dispatch}"
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

    struct ItemStack;
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
    mod generated {
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
        let mut claimed = false;
        dispatch_use_item_on(&mut host, &mods, &ItemStack, [0; 3], [0; 3], &mut claimed);
        assert!(claimed);
        assert_eq!(*calls.lock().unwrap(), [1]);
        calls.lock().unwrap().clear();
        dispatch_use_item_on(&mut host, &mods, &ItemStack, [0; 3], [0; 3], &mut claimed);
        assert!(calls.lock().unwrap().is_empty());
    }
}
