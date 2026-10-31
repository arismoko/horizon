#!/usr/bin/env python3
"""Vanilla's server management protocol (JSON-RPC over a WebSocket), Horizon's against vanilla's.

The pinned vanilla server (`net.minecraft.server.Main nogui`) and Horizon's (`horizon-server run`, which
serves the protocol beside the game it supervises) start with the same `management-server-*` settings (a
fixed secret, TLS off, one allowed origin). Each is then sent the same things, and every answer is compared
exactly:

- HTTP requests without the secret, with a wrong one, and with it in the WebSocket subprotocol from an
  origin that isn't allowed: the status and the JSON body;
- the handshake with the secret in the subprotocol from the allowed origin: the subprotocol answered;
- over a WebSocket with the bearer token, a sequence of messages: malformed JSON, a bare number, unknown and
  unparsable method names, a bad request id, wrong parameters, a notification (no answer), a batch, an
  answer to nothing, and `rpc.discover` (its document must also equal `reports/json-rpc-api-schema.json`
  from vanilla's data generator).

A parse error's `data` is Gson's message in vanilla and serde's in Horizon: left out. The notifications vanilla
sends meanwhile (`server/started`, `server/saving`...) are set aside: Horizon doesn't send them yet.
"""
import base64
import json
import os
from pathlib import Path
import shutil
import socket
import struct
import subprocess
import sys
import tempfile
import time

try:
    from . import lab, oracle_harness as oh, test_rust_parity as base
    from .server_bench import DONE, Server
except ImportError:
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import lab
    import oracle_harness as oh
    import test_rust_parity as base
    from server_bench import DONE, Server

NAME = 'management-protocol-parity'
EXE = '.exe' if os.name == 'nt' else ''
SECRET = 'abcdefghijABCDEFGHIJ0123456789abcdefghij'
ORIGIN = 'http://localhost:3000'
# What both servers are sent over the socket, in order; None marks what must get no answer.
MESSAGES = [
    '{nope',
    '5',
    '{"jsonrpc":"2.0","id":1,"method":"minecraft:nope"}',
    '{"jsonrpc":"2.0","id":2,"method":"Bad Name"}',
    '{"jsonrpc":"2.0","id":{"a":1},"method":"minecraft:players"}',
    '{"jsonrpc":"2.0","id":3,"method":"rpc.discover","params":{}}',
    '{"jsonrpc":"2.0","id":4,"method":"minecraft:players","params":[1]}',
    '{"jsonrpc":"2.0","id":5,"method":"minecraft:server/save"}',
    '{"jsonrpc":"2.0","id":6,"method":"minecraft:server/save","params":{"other":true}}',
    '{"jsonrpc":"2.0","id":7,"method":"minecraft:server/save","params":[true,false]}',
    '{"jsonrpc":"2.0","method":"minecraft:server/status"}',
    '{"jsonrpc":"2.0","id":8,"result":true}',
    '{"jsonrpc":"2.0","id":9}',
    '[{"jsonrpc":"2.0","id":10,"method":"rpc.discover","params":{"x":1}},{"jsonrpc":"2.0","method":"rpc.discover"},'
    '{"jsonrpc":"2.0","id":"eleven","method":"minecraft:nope","params":[]}]',
    '[]',
    '{"jsonrpc":"2.0","id":12,"method":"rpc.discover"}',
]


def management_properties(port):
    return {
        'management-server-enabled': 'true',
        'management-server-host': '127.0.0.1',
        'management-server-port': str(port),
        'management-server-secret': SECRET,
        'management-server-tls-enabled': 'false',
        'management-server-allowed-origins': ORIGIN,
    }


def write_eula(folder):
    (folder / 'eula.txt').write_text('eula=true\n')


class Socket:
    """A WebSocket client: the handshake, masked text frames out, text frames in."""

    def __init__(self, port, headers):
        self.sock = socket.create_connection(('127.0.0.1', port), timeout=60)
        key = base64.b64encode(os.urandom(16)).decode()
        lines = [f'GET / HTTP/1.1', f'Host: 127.0.0.1:{port}', 'Upgrade: websocket', 'Connection: Upgrade',
                 f'Sec-WebSocket-Key: {key}', 'Sec-WebSocket-Version: 13', *headers]
        self.sock.sendall(('\r\n'.join(lines) + '\r\n\r\n').encode())
        self.buffer = b''
        while b'\r\n\r\n' not in self.buffer:
            data = self.sock.recv(4096)
            if not data:
                break
            self.buffer += data
        head, _, self.buffer = self.buffer.partition(b'\r\n\r\n')
        self.head = head.decode('iso-8859-1').split('\r\n')

    def header(self, name):
        for line in self.head[1:]:
            k, _, v = line.partition(':')
            if k.strip().lower() == name.lower():
                return v.strip()
        return None

    def send(self, text):
        payload = text.encode()
        mask = os.urandom(4)
        length = len(payload)
        head = bytes([0x81]) + (bytes([0x80 | length]) if length < 126 else
                                bytes([0x80 | 126]) + struct.pack('>H', length) if length < 65536 else
                                bytes([0x80 | 127]) + struct.pack('>Q', length))
        self.sock.sendall(head + mask + bytes(b ^ mask[i % 4] for i, b in enumerate(payload)))

    def _read(self, n):
        while len(self.buffer) < n:
            data = self.sock.recv(65536)
            if not data:
                raise EOFError('the server closed the socket')
            self.buffer += data
        out, self.buffer = self.buffer[:n], self.buffer[n:]
        return out

    def receive(self, timeout):
        """The next text message, or None if none comes within the timeout."""
        self.sock.settimeout(timeout)
        try:
            first = self._read(2)
        except socket.timeout:
            return None
        self.sock.settimeout(60)
        length = first[1] & 0x7f
        if length == 126:
            length = struct.unpack('>H', self._read(2))[0]
        elif length == 127:
            length = struct.unpack('>Q', self._read(8))[0]
        payload = self._read(length)
        if first[0] & 0x0f != 1:
            raise AssertionError(f'expected a text frame, got opcode {first[0] & 0x0f}')
        return payload.decode()


def http_status(port, headers):
    """The status line and body of a request the server must refuse."""
    with socket.create_connection(('127.0.0.1', port), timeout=60) as sock:
        lines = ['GET / HTTP/1.1', f'Host: 127.0.0.1:{port}', 'Upgrade: websocket', 'Connection: Upgrade',
                 'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==', 'Sec-WebSocket-Version: 13', *headers]
        sock.sendall(('\r\n'.join(lines) + '\r\n\r\n').encode())
        data = b''
        while True:
            chunk = sock.recv(4096)
            if not chunk:
                break
            data += chunk
            head, _, body = data.partition(b'\r\n\r\n')
            if head and b'Content-Length' in head:
                length = int(next(l.split(b':')[1] for l in head.split(b'\r\n')
                                  if l.lower().startswith(b'content-length')))
                if len(body) >= length:
                    break
    head, _, body = data.partition(b'\r\n\r\n')
    return head.split(b'\r\n')[0].split(b' ')[1].decode(), body.decode()


def normal(text):
    """An answer as compared: parsed, with a parse error's message left out."""
    if text is None:
        return None
    value = json.loads(text)
    if isinstance(value, dict) and value.get('error', {}).get('code') == -32700:
        value['error'].pop('data', None)
    return value


def session(port, log):
    """Everything one server answers."""
    seen = {}
    for label, headers in [('no secret', []), ('wrong secret', ['Authorization: Bearer nope']),
                           ('origin not allowed', [f'Sec-WebSocket-Protocol: minecraft-v1,{SECRET}',
                                                   'Origin: http://evil.example'])]:
        seen[label] = http_status(port, headers)
    browser = Socket(port, [f'Sec-WebSocket-Protocol: minecraft-v1, {SECRET}', f'Origin: {ORIGIN}'])
    seen['subprotocol'] = (browser.head[0].split(' ')[1], browser.header('Sec-WebSocket-Protocol'))
    browser.sock.close()
    ws = Socket(port, [f'Authorization: Bearer {SECRET}'])
    if ws.head[0].split(' ')[1] != '101':
        raise AssertionError(f'the handshake failed: {ws.head}')
    notifications = []

    def answer(message):
        """The answer to a message, the notifications that come first set aside."""
        ws.send(message)
        while True:
            text = ws.receive(timeout=3 if '"id"' not in message or '"result"' in message else 60)
            log.write(f'> {message}\n< {text}\n')
            value = normal(text)
            if isinstance(value, dict) and 'method' in value and 'id' not in value:
                notifications.append(value['method'])
                continue
            return value

    seen['answers'] = [answer(message) for message in MESSAGES]
    seen['notifications'] = notifications
    return seen


def main():
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / NAME
    shutil.rmtree(build, ignore_errors=True)
    build.mkdir(parents=True)
    oh.run_logged(['cargo', 'build', '--profile', 'fast', '-p', 'horizon-client', '--bin', 'horizon-client',
                   '-p', 'horizon-server', '--bin', 'horizon-server'], build, 'compile-rust')
    target = lab.cargo_target() / 'fast'
    work = Path(tempfile.mkdtemp(prefix='hz-management-'))
    try:
        sides = {}
        for side in ('vanilla', 'horizon'):
            folder = work / side
            folder.mkdir()
            write_eula(folder)
            port, management = lab.free_port(), lab.free_port()
            props = {'server-port': str(port), 'online-mode': 'false', 'level-type': 'minecraft:flat',
                     'generate-structures': 'false', 'view-distance': '3', 'simulation-distance': '3',
                     **management_properties(management)}
            if side == 'vanilla':
                (folder / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in props.items()))
                command = ['java', '-Xmx2g', '-Duser.timezone=UTC', '-cp',
                           os.pathsep.join(map(str, [jars['server'], *libraries])), 'net.minecraft.server.Main',
                           'nogui']
            else:
                typed = ('server-port', 'online-mode', 'view-distance', 'simulation-distance')
                toml = ''.join(f'"{k}" = {json.dumps(v)}\n' for k, v in props.items() if k not in typed)
                (folder / 'server.toml').write_text(
                    f'[server]\nport = {port}\nonline_mode = false\nview_distance = 3\nsimulation_distance = 3\n\n'
                    f'[properties]\n{toml}\n'
                    f'[engine]\npath = {json.dumps(str(target / f"horizon-client{EXE}"))}\n'
                    f'resources = {json.dumps(str(lab.RESOURCES.resolve()))}\nrestart_on_crash = false\n\n'
                    '[backups]\nevery = "0"\n\n[panel]\nbind = ""\n')
                command = [target / f'horizon-server{EXE}', '--dir', folder, 'run']
            server = Server(command, folder, build / f'{side}.log')
            try:
                server.wait_for(DONE, 600)
                with (build / f'{side}-session.log').open('w') as log:
                    sides[side] = session(management, log)
            finally:
                server.stop()
        vanilla, horizon = sides['vanilla'], sides['horizon']
        failures = []
        for key in ('no secret', 'wrong secret', 'origin not allowed', 'subprotocol'):
            if vanilla[key] != horizon[key]:
                failures.append(f'{key}: vanilla {vanilla[key]!r}, Horizon {horizon[key]!r}')
        for message, v, h in zip(MESSAGES, vanilla['answers'], horizon['answers']):
            if v != h:
                failures.append(f'{message}\n  vanilla {json.dumps(v)[:400]}\n  Horizon {json.dumps(h)[:400]}')
        report = lab.LOCAL / 'datagen/out/reports/json-rpc-api-schema.json'
        discovered = horizon['answers'][MESSAGES.index('{"jsonrpc":"2.0","id":12,"method":"rpc.discover"}')]
        if json.loads(report.read_text()) != discovered.get('result'):
            failures.append(f'rpc.discover differs from {report}')
        (build / 'report.json').write_text(json.dumps(sides, indent=1))
        if failures:
            sys.exit(f'{NAME}: {len(failures)} differences (logs in {build}):\n' + '\n'.join(failures))
    finally:
        shutil.rmtree(work, ignore_errors=True)
    print(f'{NAME}: {len(MESSAGES) + 4} exchanges answered as vanilla answers them; rpc.discover matches the report')


if __name__ == '__main__':
    main()
