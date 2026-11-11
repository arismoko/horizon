"""E: fixed-base API points and native replacement, one shared entry cell per function."""
from pathlib import Path
import shutil,subprocess,json,re
root=Path(__file__).resolve().parents[2];base=root/'target/wholegame/kit-out/stage-B';dest=base.parent/'stage-E'
assert not dest.exists()
shutil.copytree(base,dest,ignore=shutil.ignore_patterns('target','zc-game','build.json'))
subprocess.run(['git','init','-q'],cwd=dest,check=True)
patches=root/'target/wholegame/kit/points-tick/patches'
subprocess.run(['git','apply','--reverse',str(patches/'instrumentation.patch')],cwd=dest,check=True)
p=dest/'crates/horizon-mod-api-gen/src/game_modules.rs';s=p.read_text();s=s.replace('if method.sig.ident == "vehicle_has_other_viewers"','if (method.sig.ident == "vehicle_has_other_viewers" || method.sig.ident.to_string().starts_with("__fnr_") || method.sig.ident.to_string().starts_with("__pointa_"))');p.write_text(s)
normal=lambda s:re.sub(r'<[^>]*>','',s).replace(' ','')
pts=json.load(open(base/'points.json'))['points'];targets={normal(p['target']) for p in pts}
text=(root/'target/wholegame/C-source/crates/horizon-mod-api-gen/replaceable.txt').read_text()
keep=[s for s in text.splitlines() if s.startswith('#') or (s and normal(s) in targets)]
(dest/'crates/horizon-mod-api-gen/replaceable.txt').write_text('\n'.join(keep)+'\n')
subprocess.run([str(root/'target/wholegame/pointer-api-gen-target/release/fnr-api-stage'),str(dest)],check=True)
# Exact expansion from Option C head 11091819d7; fixed-base records/dispatch stay unchanged.
shutil.copy2(root/'target/wholegame/api-reference-point.rs',dest/'crates/horizon-mod-api-macros/src/point.rs')
p=dest/'crates/horizon-mod-point-shape/src/lib.rs';s=p.read_text()
s=s.replace('pub struct Shape {','pub struct Shape {\n    pub slot: Option<usize>,',1)
s=s.replace('    let mut raw_only = false;','    let mut raw_only = false;\n    let mut slot = None;',1)
s=s.replace('        if meta.path.is_ident("raw") {','        if meta.path.is_ident("slot") {\n            slot = Some(meta.value()?.parse::<syn::LitInt>()?.base10_parse()?);\n        } else if meta.path.is_ident("raw") {',1)
s=s.replace('    Ok(Shape {\n        name,','    Ok(Shape {\n        slot,\n        name,',1);p.write_text(s)
# Arm points after the original registry is sealed; it publishes dispatchers into the same cells.
p=dest/'crates/horizon-client/src/game.rs';s=p.read_text();anchor='    mod_points::__private::seal();';assert s.count(anchor)==1;s=s.replace(anchor,anchor+'\n    crate::__fnr_arm();',1);p.write_text(s)
subprocess.run(['git','apply',str(patches/'instrumentation.patch')],cwd=dest,check=True)
