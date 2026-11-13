"""Apply explicit payload API boundary policies after the running gate ends."""
from pathlib import Path
root = Path(__file__).resolve().parents[2] / 'crates/horizon-mod-vocab/src'
reasons = {
 'PayloadBytes': 'the single public field is the opaque-byte constructor contract.',
 'PayloadId': 'the single public field is the channel constructor contract; registration owns validation.',
 'PayloadProtocol': 'the closed registration protocol set is configuration, play, or their union.',
 'PayloadDirection': 'the closed registration flow set is clientbound, serverbound, or both.',
 'HandlerThread': 'the two executors define the complete world-access scheduling contract.',
 'PayloadDeclaration': 'callers construct the complete compatibility and scheduling record explicitly.',
 'PayloadAccepted': 'local queue acceptance is the only success state, never a remote acknowledgement.',
 'PayloadFrameStage': 'physical framing has exactly encoded and pre-compression bounds.',
 'PayloadSendError': 'callers can handle every explicit local refusal in the closed Wire result schema.',
 'PayloadContextError': 'the closed Wire result schema enumerates all context capability refusals.',
 'PayloadWorkStatus': 'work either completes in the current root or queues on the receiving main thread.',
}
p = root / 'payload.rs'
s = p.read_text()
for name, reason in reasons.items():
 kind = 'struct' if name in {'PayloadBytes', 'PayloadId', 'PayloadDeclaration'} else 'enum'
 old = f'pub {kind} {name}'
 assert s.count(old) == 1, old
 s = s.replace(old, f'// exhaustive: {reason}\n{old}')
p.write_text(s)
for rel, names in [('parts/payloads.rs', ['Payloads', 'PayloadReceiver']), ('payload/registrar.rs', ['PayloadRegistrar', 'RegisterPayloadHandlersEvent', 'RegisterClientPayloadHandlersEvent'])]:
 p = root / rel
 s = p.read_text()
 for name in names:
  old = f'pub struct {name}'
  assert s.count(old) == 1, old
  s = s.replace(old, f'#[non_exhaustive]\n{old}')
 if rel == 'parts/payloads.rs':
  old = '#[doc(hidden)]\npub fn installed_payloads'
  assert s.count(old) == 1
  s = s.replace(old, '// hidden: engine registry accessor exported only through __private::game.\n' + old)
 p.write_text(s)
