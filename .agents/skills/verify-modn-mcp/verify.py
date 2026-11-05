"""Drive the real Modn MCP stdio surface with disposable files only."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import queue
import shutil
import subprocess
import tempfile
import threading
import time
import zipfile


def snapshot(root):
    return {str(p.relative_to(root)): ('file', hashlib.sha256(p.read_bytes()).hexdigest())
            if p.is_file() else ('directory', None) for p in root.rglob('*')}


def alive(pid):
    if os.name == 'nt':
        import ctypes
        kernel = ctypes.windll.kernel32
        kernel.OpenProcess.restype = ctypes.c_void_p
        handle = kernel.OpenProcess(0x1000, False, pid)
        if not handle:
            return False
        code = ctypes.c_ulong()
        kernel.GetExitCodeProcess(ctypes.c_void_p(handle), ctypes.byref(code))
        kernel.CloseHandle(ctypes.c_void_p(handle))
        return code.value == 259
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False


class Client:
    evidence = None

    def __init__(self, server, backend, state, root, writes=False, timeout_seconds=5):
        args = [str(server), '--backend', str(backend), '--state-dir', str(state),
                '--allow-root', str(root), '--timeout-seconds', str(timeout_seconds)]
        if writes:
            args.append('--allow-writes')
        self.process = subprocess.Popen(args, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=subprocess.PIPE)
        self.lines = queue.Queue()
        self.trace = []
        self.next_id = 1
        threading.Thread(target=self.read, daemon=True).start()

    def read(self):
        for line in self.process.stdout:
            self.lines.put(line)
        self.lines.put(None)

    def raw(self, data):
        self.process.stdin.write(data)
        self.process.stdin.flush()

    def receive(self):
        line = self.lines.get(timeout=8)
        assert line is not None, self.process.stderr.read().decode()
        assert len(line) <= 524289, len(line)
        value = json.loads(line)
        self.trace.append(value)
        with (self.evidence / 'responses.jsonl').open('a') as evidence:
            evidence.write(json.dumps(value) + '\n')
        return value

    def request(self, method, params=None, identifier=None):
        identifier = self.next_id if identifier is None else identifier
        self.next_id += 1
        message = {'jsonrpc': '2.0', 'id': identifier, 'method': method}
        if params is not None:
            message['params'] = params
        self.raw(json.dumps(message).encode() + b'\n')
        response = self.receive()
        assert response['id'] == identifier, response
        return response

    def initialize(self):
        response = self.request('initialize', {'protocolVersion': '2025-06-18',
            'capabilities': {}, 'clientInfo': {'name': 'modn-fixture-client', 'version': '1'}}, 'init')
        assert response['result']['protocolVersion'] == '2025-06-18'
        self.raw(b'{"jsonrpc":"2.0","method":"notifications/initialized"}\n')

    def call(self, name, arguments):
        return self.request('tools/call', {'name': name, 'arguments': arguments})

    def close(self):
        self.process.stdin.close()
        code = self.process.wait(timeout=8)
        assert code == 0, (code, self.process.stderr.read().decode())
        return code


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--server', required=True, type=Path)
    parser.add_argument('--backend', required=True, type=Path)
    parser.add_argument('--fault-backend', required=True, type=Path)
    parser.add_argument('--evidence', required=True, type=Path)
    options = parser.parse_args()
    options.evidence.mkdir(parents=True, exist_ok=True)
    Client.evidence = options.evidence
    repo = Path(__file__).resolve().parents[3]
    checks = []
    transcripts = []

    def check(name, condition):
        (options.evidence / 'progress.json').write_text(json.dumps({'checks': checks,
            'count': len(checks), 'failed': None if condition else name}, indent=2))
        assert condition, name
        checks.append(name)

    with tempfile.TemporaryDirectory(prefix='modn-mcp-') as temporary:
        root = Path(temporary).resolve()
        state = root / 'data' / 'modn'
        state.mkdir(parents=True)
        shutil.copytree(repo / 'buckets' / 'main', state / 'buckets' / 'main')
        game = root / 'game'
        (game / 'bin' / 'x64').mkdir(parents=True)
        (game / 'bin' / 'x64' / 'Cyberpunk2077.exe').write_bytes(b'disposable fixture')
        archive = root / 'example.zip'
        with zipfile.ZipFile(archive, 'w') as package:
            package.writestr('archive/pc/mod/mcp-fixture.archive', b'fixture content')
        environment = dict(os.environ, LOCALAPPDATA=str(state.parent))
        registered = subprocess.run([str(options.backend), '--state-dir', str(state), '--json',
            'game', 'add', 'cyberpunk-2077', '--path', str(game)], env=environment,
            capture_output=True, timeout=15)
        check('real CLI registration exit 0', registered.returncode == 0)
        before = snapshot(root)
        client = Client(options.server, options.backend, state, root)
        check('pre-initialize tools refused', client.request('tools/list')['error']['code'] == -32600)
        client.initialize()
        check('ping string ID preserved', client.request('ping', identifier='ping')['result'] == {})
        check('integer zero ID preserved', client.request('ping', identifier=0)['id'] == 0)
        tools = client.request('tools/list')['result']['tools']
        check('five typed default tools', len(tools) == 5 and all(t['inputSchema']['additionalProperties'] is False for t in tools))
        check('conservative annotations', all(t['annotations']['readOnlyHint'] is False for t in tools))
        for name, args in [('catalog_search', {}), ('registered_games', {}),
                           ('inspect_archive', {'archive': str(archive)})]:
            result = client.call(name, args)['result']
            check(name + ' real CLI success', not result['isError'])
            check(name + ' JSON content', isinstance(json.loads(result['content'][0]['text']), (dict, list)))
        install = {'archive': str(archive), 'game_path': str(game), 'game_id': 'cyberpunk-2077'}
        result = client.call('preview_install', install)['result']
        check('real install preview', not result['isError'])
        check('preview preserves all state and game bytes', snapshot(root) == before)
        for name, args in [('install_archive', dict(install, approved=True)), ('unknown', {}),
                           ('registered_games', {'argv': ['recover', '--yes']}),
                           ('registered_games', []),
                           ('registered_games', None),
                           ('registered_games', True),
                           ('registered_games', 1),
                           ('registered_games', 'arguments'),
                           ('catalog_search', {'query': '--state-dir=elsewhere'}),
                           ('inspect_archive', {'archive': str(root.parent / 'outside.zip')}),
                           ('inspect_archive', {'archive': str(root / '..' / root.name / 'example.zip')}),
                           ('preview_install', dict(install, approved=True)),
                           ('preview_install', dict(install, game_path=str(state)))]:
            check('refused ' + name + ' ' + str(args)[:70], client.call(name, args)['error']['code'] == -32602)
        client.raw(b'{bad json}\n')
        check('parse error', client.receive()['error']['code'] == -32700)
        client.raw(b'\xff\n')
        check('invalid UTF-8 error', client.receive()['error']['code'] == -32700)
        for value in [[], {}, {'jsonrpc': '2.0', 'method': 'ping', 'id': None},
                      {'jsonrpc': '2.0', 'method': 'ping', 'id': 1.5}]:
            client.raw(json.dumps(value).encode() + b'\n')
            check('invalid envelope ' + str(value), client.receive()['error']['code'] == -32600)
        client.raw(b'x' * 65537 + b'\n')
        check('oversize rejected', client.receive()['error']['code'] == -32600)
        check('oversize resynchronizes', client.request('ping')['result'] == {})
        check('unknown method', client.request('shutdown')['error']['code'] == -32601)
        for params in [[], False, True, 'parameters']:
            check('nonobject params ' + str(params), client.request('tools/list', params)['error']['code'] == -32602)
        check('pagination refused', client.request('tools/list', {'cursor': 'next'})['error']['code'] == -32602)
        client.raw(b'{"jsonrpc":"2.0","method":"tools/call","params":{"name":"registered_games","arguments":{}}}\n')
        client.raw(b'{"jsonrpc":"2.0","method":"notifications/unknown"}\n')
        check('notifications have no response', client.request('ping', identifier='after-notifications')['id'] == 'after-notifications')
        check('default writes never happened', snapshot(root) == before)
        check('default EOF exit 0', client.close() == 0)
        transcripts.extend(client.trace)

        client = Client(options.server, options.backend, state, root, True)
        client.initialize()
        check('seven opt-in tools', len(client.request('tools/list')['result']['tools']) == 7)
        check('approval required', client.call('install_archive', install)['error']['code'] == -32602)
        installed = game / 'archive' / 'pc' / 'mod' / 'mcp-fixture.archive'
        installed.parent.mkdir(parents=True)
        installed.write_bytes(b'unmanaged content')
        before = snapshot(game)
        check('approved install refuses unmanaged content', client.call('install_archive',
            dict(install, approved=True))['result']['isError'])
        check('unmanaged conflict preserves game tree', snapshot(game) == before)
        installed.unlink()
        result = client.call('install_archive', dict(install, approved=True))['result']
        check('approved real install', not result['isError'])
        installed = game / 'archive' / 'pc' / 'mod' / 'mcp-fixture.archive'
        check('installed content', installed.read_bytes() == b'fixture content')
        listed = subprocess.run([str(options.backend), '--state-dir', str(state), '--json',
            'list', '--path', str(game)], env=environment, capture_output=True, timeout=15)
        check('independent real list exit 0', listed.returncode == 0)
        packages = json.loads(listed.stdout)
        package_id = packages[0]['id']
        uninstall = {'package': package_id, 'game_path': str(game), 'game_id': 'cyberpunk-2077'}
        before = snapshot(root)
        check('real uninstall preview', not client.call('preview_uninstall', uninstall)['result']['isError'])
        check('uninstall preview preserves state', snapshot(root) == before)
        installed.write_bytes(b'changed by fixture')
        check('ownership refuses drift', client.call('uninstall_package', dict(uninstall, approved=True))['result']['isError'])
        check('drift survives refusal', installed.read_bytes() == b'changed by fixture')
        installed.write_bytes(b'fixture content')
        check('approved real uninstall', not client.call('uninstall_package', dict(uninstall, approved=True))['result']['isError'])
        check('owned file removed', not installed.exists())
        check('game marker survives', (game / 'bin' / 'x64' / 'Cyberpunk2077.exe').read_bytes() == b'disposable fixture')
        client.close()
        transcripts.extend(client.trace)

        client = Client(options.server, options.backend, state, root)
        client.initialize()
        held = root / 'held-modn'
        state.rename(held)
        redirected = root / 'redirected-state'
        redirected.mkdir()
        if os.name == 'nt':
            junction = subprocess.run(['cmd', '/c', 'mklink', '/J', str(state), str(redirected)],
                capture_output=True, timeout=8)
            assert junction.returncode == 0, junction.stderr
        else:
            state.symlink_to(redirected, target_is_directory=True)
        try:
            result = client.call('registered_games', {})['result']
            check('redirected state root refused', result['isError'] and
                'manager directory is redirected' in result['content'][0]['text'])
            check('redirected state root untouched', snapshot(redirected) == {})
        finally:
            client.close()
            if os.name == 'nt':
                state.rmdir()
            else:
                state.unlink()
            held.rename(state)
        transcripts.extend(client.trace)

        for mode, expected in [('large', 'output exceeds'), ('stderr', 'output exceeds'),
                               ('sleep', 'timed out'), ('fail', 'exit 7'), ('invalid', 'invalid JSON')]:
            (state / 'mode').write_text(mode)
            client = Client(options.server, options.fault_backend, state, root, timeout_seconds=1)
            client.initialize()
            result = client.call('registered_games', {})['result']
            check('bounded backend ' + mode, result['isError'] and expected in result['content'][0]['text'])
            client.close()
            if mode == 'sleep':
                check('timeout backend is reaped', not alive(int((state / 'pid').read_text())))
            transcripts.extend(client.trace)
        (state / 'mode').write_text('sleep')
        client = Client(options.server, options.fault_backend, state, root, timeout_seconds=1)
        client.initialize()
        client.raw(b'{"jsonrpc":"2.0","id":99,"method":"tools/call","params":{"name":"registered_games","arguments":{}}}\n')
        started = time.monotonic()
        client.close()
        check('EOF bounds active backend lifetime', time.monotonic() - started < 4)
        check('EOF leaves no backend process', not alive(int((state / 'pid').read_text())))

        client = Client(options.server, options.backend, state, root)
        client.raw(b'{"jsonrpc":"2.0","id":"final","method":"ping"}')
        client.close()
        check('EOF handles final line without newline', client.receive()['id'] == 'final')
        transcripts.extend(client.trace)

    options.evidence.mkdir(parents=True, exist_ok=True)
    (options.evidence / 'protocol.json').write_text(json.dumps(transcripts, indent=2))
    (options.evidence / 'summary.json').write_text(json.dumps({'checks': checks, 'count': len(checks),
        'failures': 0, 'exit': 0, 'fixtures_cleaned': True}, indent=2))
    print(json.dumps({'checks': len(checks), 'failures': 0, 'exit': 0, 'evidence': str(options.evidence)}))


if __name__ == '__main__':
    main()
