"""Stage explicit replacement readers for hand-expanded test points."""
import re
from pathlib import Path
root = Path(__file__).resolve().parents[2]
paths = list((root/'crates/horizon-mod-points/tests').glob('*.rs')) + [root/'crates/horizon-mod-wasm/tests/native_listener.rs', root/'crates/horizon-mod-wasm/tests/real_member.rs', root/'crates/horizon-mod-wasm/tests/support/checks.rs']
for p in paths:
    s = p.read_text(); additions=[]; arms=[]
    if 'point(slot =' in s: continue
    def point(m):
        slot = len(arms) * 2 + (6 if p.name == 'checks.rs' else 0)
        name, args, ret = m.group(2,3,4)
        types = ', '.join(a.split(':',1)[1].strip() for a in args.split(',') if a.strip())
        ty = f'fn({types}) {ret or ""}'
        for kind, cell in [('slot',slot),('body',slot+1)]:
            additions.append(f'fn __fnr_read_{kind}_{slot}() -> Option<{ty}> {{ let pointer = crate::__REPLACE[{cell}].load(std::sync::atomic::Ordering::Relaxed); if pointer.is_null() {{ None }} else {{ Some(unsafe {{ std::mem::transmute::<*mut (), {ty}>(pointer) }}) }} }}')
        arms.append(f'__pointa_{name}();')
        return m.group(0).replace('point(',f'point(slot = {slot}, ',1)
    s = re.sub(r'#\[(?:horizon_mod_api_macros::)?point\(([^\n]*)\)\]\nfn (\w+)\(([^\n]*)\)(\s*->\s*[^\{]+)?\s*\{', point, s)
    if not arms: continue
    if p.name != 'checks.rs':
        count = 8 if p.name in ('native_listener.rs', 'real_member.rs') else len(arms)*2
        additions.insert(0, f'static __REPLACE: [std::sync::atomic::AtomicPtr<()>; {count}] = [const {{ std::sync::atomic::AtomicPtr::new(std::ptr::null_mut()) }}; {count}];')
    s += '\n' + '\n'.join(additions) + '\n'
    if p.name == 'checks.rs': s=s.replace('pub fn check() {','pub fn check() {\n'+'\n'.join(arms))
    elif p.name == 'point_expansion.rs': s=s.replace('fn double(value', 'fn double(value') ; s=s.replace('POINTS[', 'POINTS[') # arming follows explicit booleans below
    else: s=s.replace('mod_points::__private::seal();','mod_points::__private::seal();\n'+'\n'.join(arms))
    p.write_text(s)
