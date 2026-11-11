from pathlib import Path
import shutil,json,difflib,subprocess
root=Path(__file__).resolve().parents[2];base=root/'target/wholegame/kit-out/stage-B';dest=root/'target/wholegame/kit-out/stage-E'
assert not dest.exists()
shutil.copytree(base,dest,ignore=shutil.ignore_patterns('target','zc-game','build.json'))
changes={}
def change(path,fn):
 p=dest/path;s=p.read_text();t=fn(s);p.write_text(t);changes[path]=(s,t)
path='crates/horizon-mod-api-macros/src/point.rs'
def macro(s):
 s=s.replace('let shape = match point_shape::shape','let mut shape = match point_shape::shape',1)
 anchor='    let name = shape.name.clone();'
 s=s.replace(anchor,'    if !shape.api() { return quote!(#function); }\n    shape.raw = false;\n'+anchor,1)
 s=s.replace('#table::__private::POINTS[','#table::__private::POINTS_SHARED[',1)
 s=s.replace('''        let listened = listened(&shape.return_variant());
        quote!(let __returned = #listened;)''','''        quote!(let __returned = __head_listened;)''',1)
 s=s.replace('''        if #head_listened {''','''        let __head_listened = #head_listened;
        if __head_listened {''',1)
 return s
change(path,macro)
points=json.load(open(base/'points.json'))['points']
camel=lambda name:''.join(p[:1].upper()+p[1:] for p in name.split('_') if p)
lines=[]
for p in points:
 if p['api_refusal'] is not None:continue
 head=camel(p['name']);value=f'POINTS[PointId::{head} as usize].load(Ordering::Relaxed)'
 if p['at_return']:value+=f' || POINTS[PointId::{head}Return as usize].load(Ordering::Relaxed)'
 lines.append(f'    POINTS_SHARED[PointId::{head} as usize].store({value}, Ordering::Release);')
def registry(s):
 s=s.replace('RegisterError, seal, stop','RegisterError, stop',1)
 return s+'''\n// Benchmark-only single flag per API-capable function, published once after registry seal.
pub static POINTS_SHARED: [AtomicBool; POINT_COUNT] = [const { AtomicBool::new(false) }; POINT_COUNT];
pub fn seal() {
    use std::sync::atomic::Ordering;
    mod_api::__private::points::seal();
'''+ '\n'.join(lines)+'\n}\n'
change('crates/horizon-mod-points/src/registry.rs',registry)
change('crates/horizon-mod-points/src/lib.rs',lambda s:s.replace('{POINTS, SITES, TABLE, seal}','{POINTS, POINTS_SHARED, SITES, TABLE, seal}',1))
patch=''.join(''.join(difflib.unified_diff(a.splitlines(True),b.splitlines(True),fromfile='a/'+p,tofile='b/'+p)) for p,(a,b) in changes.items())
(root/'target/wholegame/variant-patches/E.patch').write_text(patch)
print('API functions retained',len(lines),'of',len(points))
