"""Make independent measurement-only macro patches against kit base; never edit shared kit."""
from pathlib import Path
import difflib
root=Path(__file__).resolve().parents[2]
path='crates/horizon-mod-api-macros/src/point.rs'
base=(root/'target/wholegame/kit-source'/path).read_text()
out=root/'target/wholegame/variant-patches';out.mkdir(exist_ok=True)
def eligibility(s):
 anchor='    let name = shape.name.clone();'
 insert='''    // Measurement policy: explicit inline or <= 64 tokens with no calls/macros/loops.
    struct Leaf(bool);
    impl<'ast> syn::visit::Visit<'ast> for Leaf {
        fn visit_expr_call(&mut self, _: &'ast syn::ExprCall) { self.0 = false; }
        fn visit_expr_method_call(&mut self, _: &'ast syn::ExprMethodCall) { self.0 = false; }
        fn visit_expr_macro(&mut self, _: &'ast syn::ExprMacro) { self.0 = false; }
        fn visit_expr_loop(&mut self, _: &'ast syn::ExprLoop) { self.0 = false; }
        fn visit_expr_for_loop(&mut self, _: &'ast syn::ExprForLoop) { self.0 = false; }
        fn visit_expr_while(&mut self, _: &'ast syn::ExprWhile) { self.0 = false; }
    }
    let mut leaf = Leaf(true);
    syn::visit::Visit::visit_block(&mut leaf, &function.block);
    let block = &function.block;
    let small = quote!(#block).to_string().split_whitespace().count() <= 64;
    let explicit = function.attrs.iter().any(|a| a.path().is_ident("inline"));
    if explicit || (small && leaf.0) {
        eprintln!("HZ_SITE_SKIP {}", shape.name);
        return quote!(#function);
    }
    eprintln!("HZ_SITE_KEEP {}", shape.name);
'''
 assert anchor in s;return s.replace(anchor,insert+anchor,1)
def plain(s):
 old='.load(::std::sync::atomic::Ordering::Acquire)'
 assert s.count(old)==1
 return s.replace(old,'.as_ptr().read()',1).replace('quote!(#table::__private::POINTS[','quote!(unsafe { #table::__private::POINTS[',1).replace('.as_ptr().read());','.as_ptr().read() });',1)
def borrowed(s):
 old='''            (#(#owned,)*) = ::mod_api::__private::points::cold(|| { #head_api #head_raw (#(#owned,)*) });'''
 # Option storage gives the cold closure only references while preserving non-Copy ownership.
 # Empty slots during a panic do not duplicate or leak values.
 new='''            #(let mut #owned = ::std::option::Option::Some(#owned);)*
            ::mod_api::__private::points::cold(|| {
                #(let mut #owned = #owned.take().expect("owned point argument");)*
                #head_api #head_raw
                // Return through borrowed slots rather than a by-value tuple ABI.
                (#(#owned,)*)
            });'''
 # A separate slot identifier is required; construct it in the macro expansion.
 anchor='    let cancel_local = cancels.then(|| quote!(let mut __cancel = false;));'
 extra='''    let borrowed_slots: Vec<syn::Ident> = owned.iter().map(|p| format_ident!("__borrowed_{}", p)).collect();
'''
 new='''            #(let mut #borrowed_slots = ::std::option::Option::Some(#owned);)*
            ::mod_api::__private::points::cold(|| {
                #(let mut #owned = #borrowed_slots.take().expect("owned point argument");)*
                #head_api #head_raw
                #(#borrowed_slots = ::std::option::Option::Some(#owned);)*
            });
            #(#owned = #borrowed_slots.take().expect("returned point argument");)*'''
 assert old in s and anchor in s
 return s.replace(anchor,extra+anchor,1).replace(old,new,1)
for name,ops in [('Ma',[eligibility]),('Mb',[plain]),('Mc',[borrowed]),('Mall',[eligibility,plain,borrowed])]:
 s=base
 for op in ops:s=op(s)
 (out/f'{name}.patch').write_text(''.join(difflib.unified_diff(base.splitlines(True),s.splitlines(True),fromfile='a/'+path,tofile='b/'+path)))
 print(name)
