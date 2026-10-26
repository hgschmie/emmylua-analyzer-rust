"""Workspace-pull protocol regression: python3 crates/emmylua_ls/tests/workspace_diagnostics.py <server>.

Uses only the standard library. Fixtures and server logs stay under build/.
Tests synchronize mutations using document pulls while workspace pulls remain
pending. Replies are correlated by request ID, including cancellation errors.
"""
import json
import queue
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

TIMEOUT = 20
INTERVAL = 100
REQUIRED = '---@class Provider\n---@field set_values fun()\n'
OPTIONAL = '---@class Provider\n-- ---@field set_values fun()\n'
CONSUMER = '---@type Provider\nlocal settings_table = {}\nreturn settings_table\n'


class Client:
    def __init__(self, binary, root, refresh, pull=True):
        self.root = root
        self.messages = queue.Queue()
        self.next_id = 0
        self.responses = {}
        self.versions = {}
        self.published = []
        self.refresh_count = 0
        self.configuration = {}
        self.log = (root / 'server.log').open('w')
        self.process = subprocess.Popen(
            [str(binary), '--log-path', str(root / 'logs'),
             '--resources-path', str(root.parent / 'workspace-diagnostic-resources')],
            cwd=root, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.log)
        threading.Thread(target=self.read, daemon=True).start()
        try:
            result = self.request('initialize', {
                'processId': None, 'rootUri': root.as_uri(),
                'workspaceFolders': [{'uri': root.as_uri(), 'name': 'test'}],
                'capabilities': {
                    'textDocument': {'diagnostic': {}} if pull else {},
                    'workspace': {
                        'configuration': True,
                        # Use only explicitly sent events, avoiding duplicate OS watcher events.
                        'didChangeWatchedFiles': {'dynamicRegistration': True},
                        **({'diagnostics': {'refreshSupport': refresh}} if refresh is not None else {})}}})
            assert result['capabilities']['diagnosticProvider']['workspaceDiagnostics']
            self.send('initialized', {})
            # This response is dispatched only after the server finishes initialization.
            self.diagnostic()
        except BaseException:
            self.close()
            raise

    def read(self):
        try:
            while True:
                headers = {}
                while True:
                    line = self.process.stdout.readline()
                    if not line:
                        raise EOFError('Language server closed stdout')
                    if line == b'\r\n':
                        break
                    key, value = line.decode().split(':', 1)
                    headers[key.lower()] = value.strip()
                self.messages.put(json.loads(self.process.stdout.read(int(headers['content-length']))))
        except Exception as error:
            self.messages.put(error)

    def write(self, message):
        raw = json.dumps({'jsonrpc': '2.0', **message}).encode()
        self.process.stdin.write(f'Content-Length: {len(raw)}\r\n\r\n'.encode() + raw)
        self.process.stdin.flush()

    def send(self, method, params):
        self.write({'method': method, 'params': params})

    def receive(self, timeout):
        message = self.messages.get(timeout=timeout)
        if isinstance(message, Exception):
            raise message
        if 'method' in message and 'id' in message:
            method = message['method']
            if method == 'workspace/configuration':
                result = [self.configuration if item.get('section') == 'emmylua' else None
                          for item in message['params']['items']]
            elif method == 'workspace/diagnostic/refresh':
                self.refresh_count += 1
                result = None
            elif method in ('client/registerCapability', 'client/unregisterCapability',
                            'window/workDoneProgress/create', 'window/showMessageRequest'):
                # Registration/progress results are null; null declines a rename prompt.
                result = None
            else:
                self.write({'id': message['id'], 'error': {
                    'code': -32601, 'message': f'Unsupported test-client request: {method}'}})
                raise AssertionError(f'Unexpected server request: {message}')
            self.write({'id': message['id'], 'result': result})
        elif 'method' not in message:
            self.responses[message['id']] = message
        elif message['method'] == 'textDocument/publishDiagnostics':
            self.published.append(message)
        return message

    def begin(self, method, params):
        self.next_id += 1
        self.write({'id': self.next_id, 'method': method, 'params': params})
        return self.next_id

    def response(self, request_id, timeout=TIMEOUT):
        deadline = time.monotonic() + timeout
        while request_id not in self.responses:
            try:
                self.receive(max(0.01, deadline - time.monotonic()))
            except queue.Empty:
                raise AssertionError(f'Request {request_id} timed out') from None
            assert time.monotonic() < deadline, f'Request {request_id} timed out'
        return self.responses.pop(request_id)

    def request(self, method, params):
        message = self.response(self.begin(method, params))
        assert 'error' not in message, message
        return message['result']

    def workspace(self):
        return self.begin('workspace/diagnostic', {'previousResultIds': []})

    def report(self, request_id, missing):
        response = self.response(request_id)
        assert 'error' not in response, response
        reports = response['result']['items']
        target = next(r for r in reports if r['uri'] == (self.root / 'settings.lua').as_uri())
        assert any(d.get('code') == 'missing-fields' for d in target['items']) == missing, target
        assert all(r['version'] == self.versions.get(r['uri']) for r in reports), 'incorrect document version'
        return reports

    def pending(self, request_id):
        self.pump_for(0.3)
        assert request_id not in self.responses, self.responses.get(request_id)

    def pump_for(self, duration):
        deadline = time.monotonic() + duration
        while time.monotonic() < deadline:
            try:
                self.receive(max(0.01, deadline - time.monotonic()))
            except queue.Empty:
                break

    def diagnostic(self):
        return self.request('textDocument/diagnostic', {
            'textDocument': {'uri': (self.root / 'settings.lua').as_uri()}})['items']

    def wait_missing(self, missing):
        deadline = time.monotonic() + TIMEOUT
        while True:
            diagnostics = self.diagnostic()
            if any(d.get('code') == 'missing-fields' for d in diagnostics) == missing:
                return
            assert time.monotonic() < deadline, diagnostics
            self.pump_for(0.05)

    def open(self, path, text):
        self.versions[path.as_uri()] = 10000
        self.send('textDocument/didOpen', {'textDocument': {
            'uri': path.as_uri(), 'languageId': 'lua', 'version': 10000, 'text': text}})

    def watch(self, path, kind):
        self.send('workspace/didChangeWatchedFiles', {
            'changes': [{'uri': path.as_uri(), 'type': kind}]})

    def close(self):
        if self.process.poll() is None:
            self.process.terminate()
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait(timeout=5)
        self.process.stdin.close()
        self.process.stdout.close()
        self.log.close()


def edits(c):
    types = c.root / 'types.lua'
    c.open(c.root / 'settings.lua', CONSUMER)
    c.open(types, OPTIONAL)
    c.wait_missing(False)
    # Establish a clean initial pass before exercising an unsaved edit.
    response = c.response(c.workspace())
    assert 'error' not in response, response
    for version, text, missing in [(10001, REQUIRED, True), (10002, OPTIONAL, False),
                                   (10003, REQUIRED, True)]:
        pending = c.workspace()
        c.pending(pending)
        c.versions[types.as_uri()] = version
        c.send('textDocument/didChange', {'textDocument': {'uri': types.as_uri(), 'version': version},
                                        'contentChanges': [{'text': text}]})
        c.wait_missing(missing)
        c.report(pending, missing)
    assert c.refresh_count == 0
    assert not c.published


def lifecycle(c):
    c.report(c.workspace(), False)
    pending = c.workspace()
    c.pending(pending)
    c.send('$/cancelRequest', {'id': pending})
    response = c.response(pending, timeout=2)
    assert response.get('error', {}).get('code') == -32800, response
    replaced = c.workspace()
    c.pending(replaced)
    pending = c.workspace()
    response = c.response(replaced, timeout=2)
    assert response.get('error', {}).get('code') == -32800, response
    c.pending(pending)
    c.send('textDocument/didSave', {'textDocument': {'uri': (c.root / 'settings.lua').as_uri()}})
    c.report(pending, False)
    pending = c.workspace()
    c.pending(pending)
    c.configuration = {'diagnostics': {'disable': ['missing-fields']}}
    c.send('workspace/didChangeConfiguration', {'settings': {}})
    before = c.refresh_count
    c.request('shutdown', None)
    response = c.response(pending, timeout=2)
    assert response.get('error', {}).get('code') == -32800, response
    c.pump_for(2.2)
    assert c.refresh_count == before
    c.send('exit', None)


def files(c):
    c.report(c.workspace(), False)
    types = c.root / 'types.lua'
    pending = c.workspace()
    c.watch(c.root / 'never-indexed.lua', 3)
    c.pending(pending)
    c.send('$/cancelRequest', {'id': pending})
    assert c.response(pending)['error']['code'] == -32800
    for kind, text, missing in [(2, REQUIRED, True), (2, OPTIONAL, False),
                                (2, REQUIRED, True), (3, None, False), (1, REQUIRED, True)]:
        pending = c.workspace()
        if text is None:
            types.unlink()
        else:
            types.write_text(text)
        c.watch(types, kind)
        c.wait_missing(missing)
        reports = c.report(pending, missing)
        if text is None:
            assert any(r['uri'] == types.as_uri() and not r['items'] for r in reports)
    pending = c.workspace()
    renamed = c.root / 'renamed.lua'
    types.rename(renamed)
    renamed.write_text(OPTIONAL)
    c.send('workspace/didRenameFiles', {'files': [{'oldUri': types.as_uri(), 'newUri': renamed.as_uri()}]})
    c.wait_missing(False)
    reports = c.report(pending, False)
    assert any(r['uri'] == types.as_uri() and not r['items'] for r in reports)
    assert any(r['uri'] == renamed.as_uri() for r in reports)


def close_removed(c):
    types = c.root / 'types.lua'
    c.open(types, REQUIRED)
    c.wait_missing(True)
    c.report(c.workspace(), True)
    pending = c.workspace()
    types.unlink()
    c.versions.pop(types.as_uri())
    c.send('textDocument/didClose', {'textDocument': {'uri': types.as_uri()}})
    c.wait_missing(False)
    c.report(pending, False)


def configuration(c):
    c.open(c.root / 'types.lua', REQUIRED)
    c.wait_missing(True)
    c.report(c.workspace(), True)
    for disabled in [True, False]:
        pending = c.workspace()
        c.configuration = {'diagnostics': {'disable': ['missing-fields'] if disabled else []}}
        c.send('workspace/didChangeConfiguration', {'settings': {}})
        c.wait_missing(not disabled)
        c.report(pending, not disabled)
    for disabled in [True, False]:
        pending = c.workspace()
        config = c.root / '.emmyrc.json'
        config.write_text(json.dumps({'diagnostics': {'diagnosticInterval': INTERVAL,
            'disable': ['missing-fields'] if disabled else []}}))
        c.watch(config, 2)
        c.wait_missing(not disabled)
        c.report(pending, not disabled)


def main():
    binary = Path(sys.argv[1]).resolve()
    cases = {'edits': edits, 'files': files, 'close': close_removed, 'lifecycle': lifecycle,
             'configuration': configuration}
    selected = sys.argv[2:] or list(cases)
    base = Path('build/workspace-diagnostic-tests').resolve()
    base.mkdir(parents=True, exist_ok=True)
    for name in selected:
        for refresh in ([True, False, None] if name == 'configuration' else [True]):
            root = Path(tempfile.mkdtemp(prefix=f'{name}-{refresh}-', dir=base))
            (root / 'types.lua').write_text(OPTIONAL)
            (root / 'settings.lua').write_text(CONSUMER)
            (root / '.emmyrc.json').write_text(json.dumps({'diagnostics': {'diagnosticInterval': INTERVAL}}))
            c = Client(binary, root, refresh)
            try:
                cases[name](c)
                if not refresh:
                    assert c.refresh_count == 0
                print(f'PASS {name} refresh={refresh}', flush=True)
            finally:
                c.close()


if __name__ == '__main__':
    main()
