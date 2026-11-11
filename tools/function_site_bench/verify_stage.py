"""Fail before a costly game build if a benchmark patch was skipped."""
from pathlib import Path
import json,sys
stage=Path(sys.argv[1]);v=sys.argv[2]
macro=(stage/'crates/horizon-mod-api-macros/src/point.rs').read_text()
checks={
 'tick instrumentation':'zc_started' in (stage/'crates/horizon-client/src/game/interface/dedicated/mod.rs').read_text(),
 'frame instrumentation':'zc_bench::frame()' in (stage/'crates/horizon-client/src/game/runtime/window_handler.rs').read_text(),
 'instrumentation module':(stage/'crates/horizon-client/src/game/runtime/zc_bench.rs').is_file(),
 'counter storage':'pub static COUNTS:' in (stage/'crates/horizon-mod-vocab/src/__private/points.rs').read_text(),
 'fixed-base metadata':len(json.loads((stage/'points.json').read_text())['points'])==9764,
}
if v in ['A','D']:checks['no-points expansion']='REQ-R1-2 build A: no point' in macro
if v=='C':checks['five-byte site']='0x0f, 0x1f, 0x44, 0x00, 0x00' in macro
if v=='D':checks['replacement slots']='__REPLACE' in (stage/'crates/horizon-client/src/lib.rs').read_text()
if v=='E':
 checks['API-only']='shape.raw = false' in macro and 'if !shape.api()' in macro
 checks['shared flag']='POINTS_SHARED[' in macro and 'let __returned = __head_listened' in macro
if v in ['Ma','Mall']:checks['leaf rule']='HZ_SITE_SKIP' in macro and 'fn visit_macro' in macro
if v in ['Mb','Mall']:checks['plain flag read']='.as_ptr().read()' in macro
if v in ['Mc','Mall']:checks['borrowed argument state']='borrowed_slots' in macro
for name,ok in checks.items():
 if not ok:raise SystemExit(f'{stage}: missing {name}')
print(stage,v,'verified',len(checks),'conditions')
