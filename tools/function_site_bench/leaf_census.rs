use quote::quote;
use std::{collections::BTreeSet, fs, path::Path};
use syn::visit::Visit;
struct Leaf(bool);
impl<'a> Visit<'a> for Leaf {
    fn visit_expr_call(&mut self, _: &'a syn::ExprCall) {
        self.0 = false;
    }
    fn visit_expr_method_call(&mut self, _: &'a syn::ExprMethodCall) {
        self.0 = false;
    }
    fn visit_macro(&mut self, _: &'a syn::Macro) {
        self.0 = false;
    }
    fn visit_expr_loop(&mut self, _: &'a syn::ExprLoop) {
        self.0 = false;
    }
    fn visit_expr_for_loop(&mut self, _: &'a syn::ExprForLoop) {
        self.0 = false;
    }
    fn visit_expr_while(&mut self, _: &'a syn::ExprWhile) {
        self.0 = false;
    }
}
#[derive(Default)]
struct Scan {
    skip: BTreeSet<String>,
    all: BTreeSet<String>,
}
impl Scan {
    fn function(&mut self, attrs: &[syn::Attribute], sig: &syn::Signature, block: &syn::Block) {
        let Some(attr) = attrs
            .iter()
            .find(|a| a.path().segments.last().is_some_and(|s| s.ident == "point"))
        else {
            return;
        };
        let tokens = match &attr.meta {
            syn::Meta::List(l) => l.tokens.clone(),
            _ => Default::default(),
        };
        let shape = point_shape::shape(tokens, sig).expect("point shape");
        self.all.insert(shape.name.clone());
        let mut leaf = Leaf(true);
        leaf.visit_block(block);
        let small = quote!(#block).to_string().split_whitespace().count() <= 64;
        let explicit = attrs.iter().any(|a| a.path().is_ident("inline"));
        if explicit || (small && leaf.0) {
            self.skip.insert(shape.name);
        }
    }
    fn walk(&mut self, path: &Path) {
        for e in fs::read_dir(path).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                if p.file_name().unwrap() != "target" {
                    self.walk(&p)
                }
            } else if p.extension().is_some_and(|e| e == "rs") {
                let text = fs::read_to_string(&p).unwrap();
                if let Ok(file) = syn::parse_file(&text) {
                    self.visit_file(&file);
                }
            }
        }
    }
}
impl<'a> Visit<'a> for Scan {
    fn visit_impl_item_fn(&mut self, f: &'a syn::ImplItemFn) {
        self.function(&f.attrs, &f.sig, &f.block);
        syn::visit::visit_impl_item_fn(self, f);
    }
    fn visit_item_fn(&mut self, f: &'a syn::ItemFn) {
        self.function(&f.attrs, &f.sig, &f.block);
        syn::visit::visit_item_fn(self, f);
    }
}
fn main() {
    let mut s = Scan::default();
    s.walk(Path::new(&std::env::args().nth(1).unwrap()));
    eprintln!(
        "{} total, {} dropped, {} retained",
        s.all.len(),
        s.skip.len(),
        s.all.len() - s.skip.len()
    );
    for name in s.skip {
        println!("{name}");
    }
}
