//! Characterize selected original point functions, not replacement-generated helpers.
use quote::quote;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};
use syn::{spanned::Spanned, visit::Visit};
#[derive(Deserialize)]
struct Catalog {
    points: Vec<Point>,
}
#[derive(Deserialize)]
struct Point {
    name: String,
    target: String,
}
#[derive(Default, Serialize)]
struct Features {
    calls: bool,
    macros: bool,
    loops: bool,
}
impl<'a> Visit<'a> for Features {
    fn visit_expr_call(&mut self, n: &'a syn::ExprCall) {
        self.calls = true;
        syn::visit::visit_expr_call(self, n);
    }
    fn visit_expr_method_call(&mut self, n: &'a syn::ExprMethodCall) {
        self.calls = true;
        syn::visit::visit_expr_method_call(self, n);
    }
    fn visit_macro(&mut self, _: &'a syn::Macro) {
        self.macros = true;
    }
    fn visit_expr_loop(&mut self, n: &'a syn::ExprLoop) {
        self.loops = true;
        syn::visit::visit_expr_loop(self, n);
    }
    fn visit_expr_for_loop(&mut self, n: &'a syn::ExprForLoop) {
        self.loops = true;
        syn::visit::visit_expr_for_loop(self, n);
    }
    fn visit_expr_while(&mut self, n: &'a syn::ExprWhile) {
        self.loops = true;
        syn::visit::visit_expr_while(self, n);
    }
}
#[derive(Serialize)]
struct Row {
    target: String,
    file: String,
    line: usize,
    tokens: usize,
    inline: bool,
    features: Features,
}
struct Scan {
    names: BTreeMap<String, String>,
    rows: BTreeMap<String, Row>,
    file: PathBuf,
}
fn gated(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path().is_ident("cfg") || a.path().is_ident("cfg_attr") || a.path().is_ident("test")
    })
}
impl Scan {
    fn function(&mut self, attrs: &[syn::Attribute], sig: &syn::Signature, body: &syn::Block) {
        if gated(attrs) {
            return;
        }
        let Some(attr) = attrs
            .iter()
            .find(|a| a.path().segments.last().is_some_and(|s| s.ident == "point"))
        else {
            return;
        };
        let syn::Meta::List(list) = &attr.meta else {
            return;
        };
        let shape = point_shape::shape(list.tokens.clone(), sig).expect("staged point shape");
        let Some(target) = self.names.get(&shape.name) else {
            return;
        };
        let mut features = Features::default();
        features.visit_block(body);
        let row = Row {
            target: target.clone(),
            file: self.file.display().to_string(),
            line: sig.span().start().line,
            tokens: quote!(#body).to_string().split_whitespace().count(),
            inline: attrs.iter().any(|a| a.path().is_ident("inline")),
            features,
        };
        assert!(
            self.rows.insert(target.clone(), row).is_none(),
            "duplicate source target: {target}"
        );
    }
    fn walk(&mut self, path: &Path) {
        for entry in fs::read_dir(path).unwrap() {
            let p = entry.unwrap().path();
            if p.is_dir() {
                if !matches!(
                    p.file_name().unwrap().to_str(),
                    Some("target" | "tests" | "examples" | "benches" | ".git")
                ) {
                    self.walk(&p);
                }
            } else if p.extension().is_some_and(|e| e == "rs") {
                let text = fs::read_to_string(&p).unwrap();
                if let Ok(file) = syn::parse_file(&text) {
                    self.file = p;
                    self.visit_file(&file);
                }
            }
        }
    }
}
impl<'a> Visit<'a> for Scan {
    fn visit_item_mod(&mut self, n: &'a syn::ItemMod) {
        if !gated(&n.attrs) {
            syn::visit::visit_item_mod(self, n);
        }
    }
    fn visit_item_impl(&mut self, n: &'a syn::ItemImpl) {
        if !gated(&n.attrs) {
            syn::visit::visit_item_impl(self, n);
        }
    }
    fn visit_item_fn(&mut self, n: &'a syn::ItemFn) {
        self.function(&n.attrs, &n.sig, &n.block);
    }
    fn visit_impl_item_fn(&mut self, n: &'a syn::ImplItemFn) {
        self.function(&n.attrs, &n.sig, &n.block);
    }
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let catalog: Catalog = serde_json::from_str(&fs::read_to_string(&args[2]).unwrap()).unwrap();
    let selected: BTreeSet<_> = fs::read_to_string(&args[3])
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    let names = catalog
        .points
        .into_iter()
        .filter(|p| selected.contains(&p.target))
        .map(|p| (p.name, p.target))
        .collect();
    let mut scan = Scan {
        names,
        rows: BTreeMap::new(),
        file: PathBuf::new(),
    };
    scan.walk(Path::new(&args[1]));
    assert_eq!(
        scan.rows.len(),
        selected.len(),
        "every selected target must have exactly one original source definition"
    );
    println!("{}", serde_json::to_string_pretty(&scan.rows).unwrap());
}
