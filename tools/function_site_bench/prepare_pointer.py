from pathlib import Path
import subprocess,shutil,json,re,argparse
from stage_support import exclude_generated_point_helpers
root=Path(__file__).resolve().parents[2]
parser=argparse.ArgumentParser()
parser.add_argument('--variant',choices=['D','Dw','Dh'],default='D')
parser.add_argument('--excluded',type=Path)
args=parser.parse_args()
base=root/'target/wholegame/kit-out/stage-B';stage=base.parent/f'stage-{args.variant}'
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
excluded=set(args.excluded.read_text().splitlines()) if args.excluded else set()
if excluded:
 from stage_support import mark_benchmark_exclusions
 mark_benchmark_exclusions(root,base,stage,excluded)
# Lower-crate list restricted to the fixed base's point targets; no unrelated branch code.
normal=lambda s:re.sub(r'<[^>]*>','',s).replace(' ','')
points=json.load(open(base/'points.json'))['points'];targets={normal(p['target']) for p in points}
text=(root/'target/wholegame/C-source/crates/horizon-mod-api-gen/replaceable.txt').read_text()
excluded_normal={normal(t) for t in excluded}
keep=[s for s in text.splitlines() if s.startswith('#') or (s and normal(s) in targets and normal(s) not in excluded_normal)]
(stage/'crates/horizon-mod-api-gen/replaceable.txt').write_text('\n'.join(keep)+'\n')
print('lower list retained',len([s for s in keep if not s.startswith('#')]),flush=True)
subprocess.run([str(root/'target/wholegame/pointer-gen-target/release/fnr-stage'),str(stage)],check=True)
# Keep both the staged metadata and the build-time catalog free of generated helpers.
exclude_generated_point_helpers(stage)
# Keep baseline metadata intact: do not restage generated readers/installers as points.
for patch in ['instrumentation.patch','A-point-macro.patch']:
 subprocess.run(['git','apply',str(patches/patch)],cwd=stage,check=True)
