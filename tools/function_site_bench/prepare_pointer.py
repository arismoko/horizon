from pathlib import Path
import subprocess,shutil,json,re,os
root=Path(__file__).resolve().parents[2]
src=root/'target/wholegame/kit-source';stage=root/'target/wholegame/kit-out/stage-D'
assert not stage.exists()
# The old action-area checker must accept generated installer/read helpers for its one
# otherwise-allowed associated Game function in shared.rs. No gameplay code changes.
p=src/'crates/horizon-mod-api-gen/src/game_modules.rs'
s=p.read_text()
if 'starts_with("__fnr_")' not in s:
 s=s.replace('if method.sig.ident == "vehicle_has_other_viewers"', 'if (method.sig.ident == "vehicle_has_other_viewers" || method.sig.ident.to_string().starts_with("__fnr_"))')
 p.write_text(s)
for name in subprocess.check_output(['git','ls-files','-z'],cwd=src,text=True).split('\0'):
 if name and (src/name).is_file():
  (stage/name).parent.mkdir(parents=True,exist_ok=True);shutil.copy2(src/name,stage/name)
# Lower-crate list is filtered to the same base's discoverable point targets.
normal=lambda s:re.sub(r'<[^>]*>','',s).replace(' ','')
points=json.load(open(root/'target/wholegame/kit-out/stage-B/points.json'))['points']
targets={normal(p['target']) for p in points}
text=(root/'target/wholegame/C-source/crates/horizon-mod-api-gen/replaceable.txt').read_text()
keep=[s for s in text.splitlines() if s.startswith('#') or (s and normal(s) in targets)]
(stage/'crates/horizon-mod-api-gen/replaceable.txt').write_text('\n'.join(keep)+'\n')
print('lower list retained',len([s for s in keep if not s.startswith('#')]),flush=True)
subprocess.run([str(root/'target/wholegame/pointer-gen-target/release/fnr-stage'),str(stage)],check=True)
# Main's generator adds exactly its original point metadata and exposure. A expansion removes all points.
env=dict(os.environ,CARGO_TARGET_DIR=str(root/'target/wholegame/kit-gen-tools'))
subprocess.run(['cargo','run','--release','--locked','-p','horizon-mod-api-gen','--bin','horizon-stage','--',str(stage),str(stage/'points.json'),str(stage/'internals.json'),'0.2'],cwd=src,env=env,check=True)
with (stage/'crates/horizon-mod-wasm/Cargo.toml').open('a') as f:f.write('\n[lib]\ncrate-type = ["rlib", "dylib"]\n')
for patch in ['instrumentation.patch','A-point-macro.patch']:
 subprocess.run(['git','apply',str(root/'target/wholegame/kit/points-tick/patches'/patch)],cwd=stage,check=True)
