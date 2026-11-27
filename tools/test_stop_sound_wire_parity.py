#!/usr/bin/env python3
"""Stop-sound packet bytes and selections against the native Java codec."""
import json
import os
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab, test_rust_parity as base, oracle_harness as harness


def main():
    build=lab.LOCAL/'stop-sound-wire-parity'; build.mkdir(exist_ok=True)
    (build/'report.json').unlink(missing_ok=True)
    lock,jars,libraries=base.verify_artifacts()
    cp=os.pathsep.join(map(str,[jars['server'],*libraries])); classes=build/'classes';classes.mkdir(exist_ok=True)
    source=lab.ROOT/'mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaStopSoundWireOracle.java'
    sources=[source,lab.ROOT/'crates/sunrise-protocol/src/packets/play.rs',lab.ROOT/'crates/sunrise-gameplay-common/src/game/network/remote/mod.rs']
    tested=base.source_hashes(sources)
    run=lambda cmd,name:harness.run_logged(cmd,build,name)
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-cp',cp,'-d',classes,source],'compile-java')
    run(['java','-cp',str(classes)+os.pathsep+cp,'dev.sunriselab.parity.VanillaStopSoundWireOracle',build/'java.jsonl'],'java')
    run(['cargo','build','-p','sunrise-protocol','--example','stop_sound_probe'],'compile-rust')
    run([lab.cargo_target()/'debug/examples/stop_sound_probe',build/'java.jsonl',build/'rust.jsonl'],'rust')
    java=[json.loads(x) for x in (build/'java.jsonl').read_text().splitlines()]
    rust=[json.loads(x) for x in (build/'rust.jsonl').read_text().splitlines()]
    assert len(java)==len(rust)==36
    assert all({'source':a.get('source'),'sound':a.get('sound')}==b for a,b in zip(java,rust))
    base.ensure_sources_unchanged(sources,tested)
    lab.write_json(build/'report.json',{'pass':True,'cases':len(java),'sources_sha256':tested})
    print(f'Stop sound: {len(java)} native wire selections match and re-encode byte for byte')


if __name__=='__main__':main()
