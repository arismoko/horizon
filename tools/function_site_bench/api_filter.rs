//! Measurement-only filtering of fixed-base point attributes before shared-cell staging.
use std::{fs, path::Path};
use syn::{spanned::Spanned, visit::Visit};
#[derive(Default)]
struct Filter {
    remove: Vec<proc_macro2::Span>,
    kept: usize,
    unsupported_impl: bool,
}
impl Filter {
    fn function(&mut self, attrs: &[syn::Attribute], sig: &syn::Signature) {
        for a in attrs {
            if a.path().segments.last().is_some_and(|s| s.ident == "point") {
                let tokens = match &a.meta {
                    syn::Meta::List(l) => l.tokens.clone(),
                    _ => Default::default(),
                };
                let shape = point_shape::shape(tokens, sig).expect("fixed-base point attribute");
                if !self.unsupported_impl && shape.api() && crate::replace::eligible(attrs, sig) {
                    self.kept += 1;
                } else {
                    self.remove.push(a.span());
                }
            }
        }
    }
}
impl<'a> Visit<'a> for Filter {
    fn visit_item_impl(&mut self, block: &'a syn::ItemImpl) {
        let previous = self.unsupported_impl;
        self.unsupported_impl |= block
            .generics
            .params
            .iter()
            .any(|p| !matches!(p, syn::GenericParam::Lifetime(_)))
            || !matches!(&*block.self_ty, syn::Type::Path(p) if p.qself.is_none());
        syn::visit::visit_item_impl(self, block);
        self.unsupported_impl = previous;
    }
    fn visit_item_fn(&mut self, f: &'a syn::ItemFn) {
        self.function(&f.attrs, &f.sig);
        syn::visit::visit_item_fn(self, f);
    }
    fn visit_impl_item_fn(&mut self, f: &'a syn::ImplItemFn) {
        self.function(&f.attrs, &f.sig);
        syn::visit::visit_impl_item_fn(self, f);
    }
}
pub fn filter(root: &Path) -> Result<(usize, usize), String> {
    let mut dirs = vec![root.join("crates")];
    let (mut kept, mut removed) = (0, 0);
    while let Some(dir) = dirs.pop() {
        for e in fs::read_dir(&dir).map_err(|e| e.to_string())? {
            let path = e.map_err(|e| e.to_string())?.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n != "target") {
                    dirs.push(path);
                }
                continue;
            }
            if !path.extension().is_some_and(|n| n == "rs") {
                continue;
            }
            let mut text = fs::read_to_string(&path).map_err(|e| e.to_string())?;
            let Ok(file) = syn::parse_file(&text) else {
                continue;
            };
            let mut f = Filter::default();
            f.visit_file(&file);
            kept += f.kept;
            removed += f.remove.len();
            if f.remove.is_empty() {
                continue;
            }
            let mut starts = vec![0];
            for (i, b) in text.bytes().enumerate() {
                if b == b'\n' {
                    starts.push(i + 1);
                }
            }
            let mut edits: Vec<_> = f
                .remove
                .into_iter()
                .map(|s| {
                    let a = s.start();
                    let b = s.end();
                    (starts[a.line - 1] + a.column, starts[b.line - 1] + b.column)
                })
                .collect();
            edits.sort_unstable();
            for (a, b) in edits.into_iter().rev() {
                let blank: String = text[a..b]
                    .chars()
                    .map(|c| if c == '\n' { '\n' } else { ' ' })
                    .collect();
                text.replace_range(a..b, &blank);
            }
            fs::write(&path, text).map_err(|e| e.to_string())?;
        }
    }
    Ok((kept, removed))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generic_owner_cannot_leave_an_api_point_without_a_cell() {
        let file = syn::parse_file(
            r#"
            impl<T> Holder<T> {
                #[point(name = "generic_new")]
                fn new(count: usize) -> Self { todo!() }
            }
            #[point(name = "plain")]
            fn plain(count: usize) -> usize { count }
        "#,
        )
        .unwrap();
        let mut filter = Filter::default();
        filter.visit_file(&file);
        assert_eq!(filter.remove.len(), 1);
        assert_eq!(filter.kept, 1);
    }
}
