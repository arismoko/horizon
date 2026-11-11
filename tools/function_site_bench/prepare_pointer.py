from pathlib import Path
import subprocess,shutil,json,re
from stage_support import exclude_generated_point_helpers
root=Path(__file__).resolve().parents[2]
base=root/'target/wholegame/kit-out/stage-B';stage=root/'target/wholegame/kit-out/stage-D'
assert not stage.exists()
shutil.copytree(base,stage,ignore=shutil.ignore_patterns('target','zc-game','build.json'))
# Nested target trees otherwise inherit the outer Git worktree, and git-format patches
# may silently skip every path. Make this disposable staged tree its own repository.
subprocess.run(['git','init','-q'],cwd=stage,check=True)
# Remove measurement helpers before pointer staging so they acquire no replacement sites.
patches=root/'target/wholegame/kit/points-tick/patches'
subprocess.run(['git','apply','--reverse',str(patches/'instrumentation.patch')],cwd=stage,check=True)
# Permit generated readers/installers beside the one allowed shared Game associated function.
p=stage/'crates/horizon-mod-api-gen/src/game_modules.rs';s=p.read_text()
s=s.replace('if method.sig.ident == "vehicle_has_other_viewers"','if (method.sig.ident == "vehicle_has_other_viewers" || method.sig.ident.to_string().starts_with("__fnr_"))')
p.write_text(s)
# Lower-crate list restricted to the fixed base's point targets; no unrelated branch code.
normal=lambda s:re.sub(r'<[^>]*>','',s).replace(' ','')
points=json.load(open(base/'points.json'))['points'];targets={normal(p['target']) for p in points}
text=(root/'target/wholegame/C-source/crates/horizon-mod-api-gen/replaceable.txt').read_text()
keep=[s for s in text.splitlines() if s.startswith('#') or (s and normal(s) in targets)]
(stage/'crates/horizon-mod-api-gen/replaceable.txt').write_text('\n'.join(keep)+'\n')
print('lower list retained',len([s for s in keep if not s.startswith('#')]),flush=True)
subprocess.run([str(root/'target/wholegame/pointer-gen-target/release/fnr-stage'),str(stage)],check=True)
# Keep both the staged metadata and the build-time catalog free of generated helpers.
exclude_generated_point_helpers(stage)
# Keep baseline metadata intact: do not restage generated readers/installers as points.
for patch in ['instrumentation.patch','A-point-macro.patch']:
 subprocess.run(['git','apply',str(patches/patch)],cwd=stage,check=True)
