#!/usr/bin/env python3
"""Remove the superseded envelope transport; run from the repository root.

The private recipe-book channel and generic send_packet API are deliberately kept.
Follow-up manual changes give books their own configuration capability.
"""
from pathlib import Path
import re


def change(path, transform):
    p = Path(path)
    old = p.read_text()
    new = transform(old)
    assert new != old, path
    p.write_text(new)


def cut(s, first, last):
    a = s.index(first)
    b = s.index(last, a)
    return s[:a] + s[b:]


root = 'crates/horizon-client/src/'
vocab = 'crates/horizon-mod-vocab/src/'
protocol = 'crates/horizon-protocol/src/'
for path in [protocol + 'mod_net.rs', vocab + 'hooks/net_from_client.rs', vocab + 'hooks/net_from_server.rs']:
    Path(path).unlink()
change(protocol + 'lib.rs', lambda s: s.replace('pub mod mod_net;\n', ''))
change(root + 'game/modding/native_net.rs', lambda s: (
    '//! Private custom-book connection state and client network services.\nuse super::Game;\n'
    'use horizon_protocol::packets::{configuration::CustomPayload, play::Serverbound as S};\n'
    'use mod_api::{SendPacketError, ServerboundPlayPacket};\nuse std::collections::HashMap;\n'
    '#[derive(Default)]\npub(super) struct NativeNet {\n'
    + s[s.index('    pub(super) placement:'):s.index('    from_players:')]
    + '}\n\nimpl Game<\'_> {\n'
    + s[s.index('    /// A mod\'s chat line or command'):]))
change(vocab + 'host/network.rs', lambda s: re.sub(
    r'    /// Side: client\.\n    /// The client half:.*?fn net_send_to_server\([^;]+;\n', '',
    cut(s, '/// The server host', '/// The client host'), flags=re.S))
change(root + 'game/native_hooks/base.rs', lambda s: cut(s, '    /// A message from `player`\'s client on `channel`', '    /// `dimension`\'s level opened'))
for name in ['game/interface/dedicated/mod.rs', 'game/world/integrated.rs', 'game/network/lan/mod.rs']:
    change(root + name, lambda s: s.replace('        self.flush_native_net();\n', '').replace(
        '        if self.native_net_waiting() {\n            self.run_native_net();\n        }\n', ''))
change(root + 'game/network/lan_actions/packets.rs', lambda s: re.sub(
    r'            S::CustomPayload\(p\) if p.channel == horizon_protocol::mod_net::CHANNEL => \{\n                self.guest_native_net\(conn, &p.data\)\n            }\n', '', s))
change(root + 'game/modding/native_book_net.rs', lambda s: cut(s.replace(
    '        self.native_net.to_server = 0;\n', ''),
    '    pub(super) fn is_book_or_mod_channel', '    pub(super) fn clear_guest_book_connection'))
change(root + 'lan/payload_bounds_tests.rs', lambda s: re.sub(
    r'        \("horizon:mod_net", \|body\| \{\n            protocol::mod_net::decode\(body\).is_err\(\)\n        }\),\n', '', s).replace('Rejects); 12]', 'Rejects); 11]'))
# Mechanical capability identifier changes; its codec and advertisement are corrected manually.
for name in ['net.rs', 'game/network/remote/mod.rs', 'game/modding/native_book_net.rs',
             'game/native_hooks/shared/book_screen/lifecycle_tests.rs']:
    change(root + name, lambda s: s.replace('ModNet', 'CustomBooks').replace('.mod_net', '.custom_books')
           .replace('pub mod_net:', 'pub custom_books:').replace('            mod_net:', '            custom_books:'))
change(vocab + 'hooks/notes.rs', lambda s: cut(s, '### Mod messages\n', '### Chat and commands a client mod sends'))
# Removed symbols must not remain as live catalog/ledger claims.
for path in ['docs/paper-ledger.md', 'docs/fabric-ledger.md', 'tools/refactors/api-pr2c/forwarder-renames.tsv']:
    change(path, lambda s: ''.join(line for line in s.splitlines(True)
        if not any(word in line for word in ['net_from_client', 'native_net_send_to_'])))
for path in [vocab + 'hooks/player_logged_in.rs', vocab + 'hooks/player_start_tracking.rs',
             vocab + 'hooks/notes.rs', root + 'game/native_hooks/base.rs']:
    change(path, lambda s: re.sub(r'`(?:native_)?net_send_to_player`', '`ServerHost::send_payload_to_player`', s))
change('tests/test_neoforge_counterparts.py', lambda s: ''.join(line for line in s.splitlines(True) if 'self.assertIn("| `net_from_client`' not in line))
