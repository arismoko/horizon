"""Slice sizes of an unpacked prebuilt package: python3 measure.py SDK api_slice.txt"""
import re,subprocess,os,sys,shutil
sdk=sys.argv[1]; names=set(open(sys.argv[2]).read().split())
groups={'runtime':[],'api':[],'engine':[]}
for root,_,fs in os.walk(sdk):
    for f in fs:
        rel=os.path.relpath(os.path.join(root,f),sdk).replace(os.sep,'/')
        m=re.match(r'(?:lib)?(.+?)-[0-9a-f]{16}\.(rmeta|so|dll|dylib)$',f)
        if re.fullmatch(r'(lib)?horizon_mod_(host|wasm)\.(so|dll|dylib)',f): groups['runtime'].append(rel)
        elif m and m.group(1) in names and rel.split('/')[0] in('deps','host'): groups['api'].append(rel)
        elif rel in('horizon-prebuilt.json','points.json','native_main.rs.template'): groups['api'].append(rel)
        else: groups['engine'].append(rel)
for name,files in groups.items():
    raw=sum(os.path.getsize(os.path.join(sdk,f)) for f in files)
    tar=f'slice-{name}.tar'
    subprocess.run(['tar','-cf',tar,'-C',sdk,*sorted(files)],check=True)
    out=[f'{name}: {len(files)} files, raw {raw/1e6:.1f} MB']
    if shutil.which('xz'): out.append(f'xz -6 {len(subprocess.run(["xz","-6","-T0","-c",tar],capture_output=True).stdout)/1e6:.1f} MB')
    if shutil.which('zstd'): out.append(f'zstd -19 {len(subprocess.run(["zstd","-19","-T0","-c",tar],capture_output=True).stdout)/1e6:.1f} MB')
    print(', '.join(out),flush=True); os.remove(tar)
