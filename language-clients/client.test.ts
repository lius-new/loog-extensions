import test from 'node:test';
import assert from 'node:assert/strict';
import { Client } from './client.ts';

function harness(initializeMessages: any[] = []) {
  const handlers = new Map<string, any>(), commands = new Map<string, any>(), configured = new Map<string, any>();
  const processes: { exit(value: any): void; killed: number }[] = [];
  const sent: any[] = [];
  const responses = new Map<string, any>();
  const settings = new Map<string, any>();
  const watches: any[] = [];
  const widgets = new Map<string, any>(), placements: any[] = [], statuses: any[] = [];
  function widget(description: any, parent?: any) {
    const value = { ...description, parent: parent?.id };
    widgets.set(value.id, value);
    const changed = () => {
      if (value.id === 'language-status-text') statuses.push({ text: value.text, color: widgets.get('language-status').background });
    };
    changed();
    return {
      id: value.id,
      update(patch: any) { Object.assign(value, patch); changed(); },
      remove() { for (const [id, node] of widgets) if (id === value.id || node.parent === value.id) widgets.delete(id); },
    };
  }
  const content = { value: 'fn main() {}' };
  const timers = new Set<{ milliseconds: number; callback(): void; cancel(): void }>();
  let listener: any;
  const reference = { document: 1n, lifecycle: 1n, path: '/work/a.rs', language: 'rust', version: 0n };
  const api: any = {
    providers: { register(_id: string, kind: string, handler: any) { handlers.set(kind, handler); }, onCommand(id: string, handler: any) { commands.set(id, handler); }, configure(value: any) { configured.set(value.provider, value); }, publishDiagnostics() {} },
    timers: { after(milliseconds: number, callback: () => void) { const timer = { milliseconds, callback() { timers.delete(timer); callback(); }, cancel() { timers.delete(timer); } }; timers.add(timer); if (milliseconds === 0) queueMicrotask(() => { if (timers.has(timer)) timer.callback(); }); return timer; } },
    workspace: { onDidSave() {}, roots: () => [{ id: 1n, path: '/work', name: 'work' }], captureDocument: () => ({ document: { ...reference }, snapshot: { text: () => content.value, release() {} } }), watchFiles(request: any, listener: any) { const watch = { request, listener, disposed: false, ready: Promise.resolve(), dispose() { this.disposed = true; } }; watches.push(watch); return watch; } },
    ui: { row: (value: any) => ({ ...value, kind: 'row' }), text: (text: string, value: any) => ({ ...value, text, kind: 'text' }), target: (id: string) => ({ publicId: id }), insertBefore(target: any, node: any, options: any) { placements.push({ target, node, options }); }, createElement: widget, create: (parent: any, description: any) => widget(description, parent) },
    onMessage(callback: any) { listener = callback; },
    processes: { spawn(_request: any, output: any) {
      let exit: any;
      const exited = new Promise(resolve => { exit = resolve; });
      const state = { exit, killed: 0 }; processes.push(state);
      return { exited, kill() { state.killed++; }, async write(bytes: Uint8Array) {
        const message = JSON.parse(new TextDecoder().decode(bytes));
        sent.push(message);
        if (message.method === 'initialize') for (const notification of initializeMessages)
          queueMicrotask(() => output({ channel: 'stdout', text: JSON.stringify({ jsonrpc: '2.0', ...notification }) }));
        if (message.method === 'initialize' || message.method === 'shutdown' || responses.has(message.method)) queueMicrotask(() => output({ channel: 'stdout', bytes: new TextEncoder().encode(JSON.stringify({ jsonrpc: '2.0', id: message.id, result: message.method === 'initialize' ? { capabilities: { textDocumentSync: 2, completionProvider: {}, definitionProvider: true, semanticTokensProvider: { full: true } } } : responses.get(message.method) ?? null })) }));
      } };
    } },
  };
  const client = new Client(api, { getSetting(key: string) { return settings.get(key); }, readFile() { return { text: '' }; }, log() {} }, { id: 'example.rust', name: 'Rust', languages: ['rust'], program: 'server', args: [], markers: ['Cargo.toml'], languageId: () => 'rust' });
  const settle = async () => { for (let i = 0; i < 30; i++) await Promise.resolve(); };
  return { api, client, handlers, commands, configured, processes, timers, sent, responses, settings, watches, widgets, placements, statuses, content, reference, message: (message: any) => listener(message), settle };
}

test('workspace symbols merge all roots without requiring an active document', async () => {
  const h = harness();
  let calls = 0;
  for (const [key, unit] of [['first', 'utf8'], ['second', 'utf16']]) h.client.sessions.set(key, {
    key, unit, stopped: false, ready: Promise.resolve(), effective: () => ({ workspaceSymbolProvider: true }),
    rpc: { async request(method: string) { assert.equal(method, 'workspace/symbol'); calls++; return [{ name: key, kind: 12, location: { uri: `file:///work/${key}.rs`, range: { start: { line: 1, character: 2 }, end: { line: 1, character: 5 } } } }]; } },
  } as any);
  const answer = await h.handlers.get('workspace-symbols')({ kind: 'workspace-symbols', request: 1n, context: { query: 'item' } });
  assert.equal(calls, 2);
  assert.deepEqual(answer.val.map((item: any) => item.name), ['first', 'second']);
  assert.deepEqual(answer.val.map((item: any) => item.location.range.unit), ['utf8', 'utf16']);
});

test('progress reports retain titles and distinguish string and numeric tokens', async () => {
  const h = harness();
  const statuses = h.statuses;
  await h.message({ tag: 'document-opened', val: h.reference });
  await h.client.documents.get(1n)!.queue;
  const session = [...h.client.sessions.values()][0]!;
  const notify = async (token: string | number, value: any) => { session.rpc.input(JSON.stringify({ jsonrpc: '2.0', method: '$/progress', params: { token, value } })); await h.settle(); };
  await notify(7, { kind: 'begin', title: 'First', message: '0/10' });
  await notify(7, { kind: 'report', message: '3/10' });
  assert.equal(statuses.at(-1).text, 'Rust: First 3/10');
  await notify('7', { kind: 'begin', title: 'Second' });
  await notify(7, { kind: 'end' });
  assert.equal(statuses.at(-1).text, 'Rust: Second');
  assert.equal(statuses.at(-1).color, 0x818cf8);
  await notify('7', { kind: 'end' });
  assert.equal(statuses.at(-1).text, 'Rust');
  await h.message({ tag: 'shutdown' });
});

test('status rows remain passive, preserve their handles and stop after the final idle session', async () => {
  const h = harness();
  await h.message({ tag: 'document-opened', val: h.reference });
  await h.client.documents.get(1n)!.queue;
  assert.ok(h.statuses.some(value => value.text === 'Rust: Starting' && value.color === 0x818cf8));
  assert.deepEqual(h.statuses.at(-1), { text: 'Rust', color: 0x10b981 });
  assert.equal(h.widgets.get('language-status').kind, 'row');
  assert.equal(h.widgets.get('language-status').width, undefined);
  assert.equal(h.widgets.get('language-status').height, 24);
  assert.equal(h.widgets.size, 4);
  assert.equal(h.placements.length, 1);
  assert.equal(h.placements[0].target.publicId, 'statusbar.cursor-position');
  const session = [...h.client.sessions.values()][0]!;
  session.rpc.input(JSON.stringify({ jsonrpc: '2.0', method: '$/progress', params: { token: 1, value: { kind: 'begin', title: 'Loading', percentage: 42 } } }));
  await h.settle();
  assert.deepEqual(h.statuses.at(-1), { text: 'Rust: Loading 42%', color: 0x818cf8 });
  assert.equal(h.placements.length, 1);
  await h.message({ tag: 'document-closed', val: h.reference });
  session.idle!.callback(); await h.settle();
  assert.deepEqual(h.statuses.at(-1), { text: 'Rust: Stopped', color: 0x71717a });
  await h.message({ tag: 'shutdown' });
  assert.equal(h.widgets.size, 0);
});

test('a process that cannot start reports an error instead of leaving a busy status', async () => {
  const h = harness();
  h.api.processes.spawn = () => { throw new Error('executable unavailable'); };
  await h.message({ tag: 'document-opened', val: h.reference });
  await h.client.documents.get(1n)!.queue.catch(() => {}); await h.settle();
  assert.deepEqual(h.statuses.at(-1), { text: 'Rust: Error · executable unavailable', color: 0xf43f5e });
  await h.message({ tag: 'shutdown' });
});

test('healthy roots cannot overwrite another root error or unfinished work', async () => {
  const h = harness();
  h.client.status('1:broken', 'error', 'workspace failed');
  h.client.status('2:healthy', 'ready');
  assert.deepEqual(h.statuses.at(-1), { text: 'Rust: Error · workspace failed', color: 0xf43f5e });
  await h.message({ tag: 'roots-changed', val: [{ id: 2n, path: '/other', name: 'other' }] });
  assert.deepEqual(h.statuses.at(-1), { text: 'Rust', color: 0x10b981 });
  h.client.status('2:busy', 'busy', 'Indexing');
  h.client.status('2:healthy', 'ready');
  assert.deepEqual(h.statuses.at(-1), { text: 'Rust: Indexing', color: 0x818cf8 });
  await h.message({ tag: 'shutdown' });
});

test('root selection respects POSIX case and preserves Windows and filesystem roots', async () => {
  const outside = harness(); outside.reference.path = '/WORK/a.rs';
  await assert.rejects(outside.message({ tag: 'document-opened', val: outside.reference }), /workspace folder/);
  assert.equal(outside.processes.length, 0);
  for (const [root, path] of [['C:/Work', 'c:/WORK/a.rs'], ['C:/', 'c:/a.rs'], ['/', '/a.rs']]) {
    const h = harness();
    h.api.workspace.roots = () => [{ id: 1n, path: root, name: 'root' }]; h.reference.path = path;
    await h.message({ tag: 'document-opened', val: h.reference }); await h.client.documents.get(1n)!.queue;
    assert.equal(h.sent.find(item => item.method === 'initialize').params.rootUri, root === '/' ? 'file:///' : `file:///${root}`);
    await h.message({ tag: 'shutdown' });
  }
});

test('workspace load errors stay visible after initialization and progress completes', async () => {
  const failure = 'Error loading workspace folders (expected 1, got 0)';
  const h = harness([{ method: 'window/showMessage', params: { type: 1, message: failure } }]);
  const statuses = h.statuses;
  await h.message({ tag: 'document-opened', val: h.reference });
  await h.client.documents.get(1n)!.queue;
  assert.deepEqual(statuses.at(-1), { text: `Rust: Error · ${failure}`, color: 0xf43f5e });
  const session = [...h.client.sessions.values()][0]!;
  for (const notification of [
    { method: '$/progress', params: { token: 1, value: { kind: 'begin', title: 'Loading' } } },
    { method: '$/progress', params: { token: 1, value: { kind: 'end' } } },
    { method: 'window/showMessage', params: { type: 3, message: 'Information' } },
  ]) {
    session.rpc.input(JSON.stringify({ jsonrpc: '2.0', ...notification }));
    await h.settle();
    assert.deepEqual(statuses.at(-1), { text: `Rust: Error · ${failure}`, color: 0xf43f5e });
  }
});

test('canonical Windows document and root paths synchronize and accept server diagnostics', async () => {
  for (const [root, source, expectedRoot] of [
    [String.raw`\\?\C:\work`, String.raw`C:\work\src\a.rs`, 'file:///C:/work'],
    [String.raw`C:\work`, String.raw`\\?\C:\work\src\a.rs`, 'file:///C:/work'],
    [String.raw`\\?\UNC\server\share\work`, String.raw`\\server\share\work\src\a.rs`, 'file://server/share/work'],
    [String.raw`\\server\share\work`, String.raw`\\?\UNC\server\share\work\src\a.rs`, 'file://server/share/work'],
  ]) {
    const h = harness();
    h.reference.path = source;
    h.api.workspace.roots = () => [{ id: 1n, name: 'work', path: root }];
    h.client.host.readFile = (_root: bigint, file: string) => {
      if (file !== 'Cargo.toml') throw new Error('not found');
      return { text: '' };
    };
    const published: any[] = [];
    h.api.providers.publishDiagnostics = (...args: any[]) => published.push(args);
    await h.message({ tag: 'document-opened', val: h.reference });
    const doc = h.client.documents.get(1n)!;
    await doc.queue;
    assert.equal(h.sent.find(message => message.method === 'initialize').params.rootUri, expectedRoot);
    assert.equal(h.sent.find(message => message.method === 'textDocument/didOpen').params.textDocument.uri, `${expectedRoot}/src/a.rs`);
    doc.session.rpc.input(JSON.stringify({ jsonrpc: '2.0', method: 'textDocument/publishDiagnostics', params: {
      uri: `${expectedRoot}/src/a.rs`, version: doc.serverVersion,
      diagnostics: [{ range: { start: { line: 0, character: 3 }, end: { line: 0, character: 7 } }, severity: 1, message: 'test error' }],
    } }));
    await h.settle();
    assert.equal(published.length, 1);
    assert.equal(published[0][1], source);
    assert.equal(published[0][3][0].message, 'test error');
  }
});

test('completion resolve uses bounded private entries, preserves defaults and never synchronizes an obsolete snapshot', async () => {
  const h = harness();
  await h.message({ tag: 'document-opened', val: h.reference });
  await h.client.documents.get(1n)!.queue;
  const session = [...h.client.sessions.values()][0]!;
  session.capabilities.completionProvider = { resolveProvider: true };
  const item = { label: 'main', detail: 'detail', data: { private: 'server' } };
  h.responses.set('textDocument/completion', { items: [item], itemDefaults: { insertTextFormat: 2, commitCharacters: ['('] } });
  const call = () => ({ document: 1n, version: h.reference.version, request: 9n, cursor: 3, context: {}, snapshot: { text: () => { throw new Error('reread an already synchronized query snapshot'); } } });
  const offered = await h.handlers.get('completion')({ ...call(), kind: 'completion' });
  assert.ok(offered.items[0].data.length < 100);
  assert.ok(!offered.items[0].data.includes('private'));
  h.responses.set('completionItem/resolve', { label: '', documentation: 'resolved' });
  const resolved = await h.handlers.get('completion-resolve')({ ...call(), kind: 'completion-resolve', item: offered.items[0] });
  assert.equal(resolved.documentation, 'resolved');
  assert.equal(resolved.label, 'main');
  assert.equal(resolved.detail, 'detail');
  assert.equal(resolved.insertTextFormat, 2);
  const request = h.sent.find(item => item.method === 'completionItem/resolve');
  assert.deepEqual(request.params.data, { private: 'server' });
  assert.equal(request.params.insertTextFormat, 2);
  assert.deepEqual(request.params.commitCharacters, ['(']);
  for (let i = 0; i < 4; i++) await h.handlers.get('completion')({ ...call(), kind: 'completion' });
  await assert.rejects(h.handlers.get('completion-resolve')({ ...call(), kind: 'completion-resolve', item: offered.items[0] }), /expired/);
  h.reference.version++;
  await assert.rejects(h.handlers.get('completion')({ ...call(), version: 0n, kind: 'completion' }), /no longer current/);
  assert.equal(h.sent.filter(item => item.method === 'textDocument/didChange').length, 0);
  await h.message({ tag: 'shutdown' });
});

test('temporary save text synchronizes at the same version and a later query restores the document', async () => {
  const h = harness();
  await h.message({ tag: 'document-opened', val: h.reference });
  const doc = h.client.documents.get(1n)!;
  await doc.queue;
  doc.session.capabilities.documentFormattingProvider = true;
  h.responses.set('textDocument/formatting', []);
  const before = doc.synchronized;
  const temporary = 'fn main() {\n}\n';
  const call = { document: 1n, version: h.reference.version, request: 1n, cursor: 3, context: {}, snapshot: { text: () => temporary } };
  await h.handlers.get('formatting')({ ...call, kind: 'formatting' });
  assert.equal(doc.text.value, temporary);
  assert.notEqual(doc.synchronized, before);
  h.responses.set('textDocument/definition', []);
  await h.handlers.get('definition')({ ...call, kind: 'definition' });
  assert.equal(doc.text.value, h.content.value);
  assert.equal(h.sent.filter(item => item.method === 'textDocument/didChange').length, 2);
  assert.equal(h.reference.version, 0n);
  await h.message({ tag: 'shutdown' });
});

test('a provider waiting behind document synchronization never restores its obsolete capture', async () => {
  const h = harness();
  await h.message({ tag: 'document-opened', val: h.reference });
  const doc = h.client.documents.get(1n)!;
  await doc.queue;
  let resume!: () => void;
  doc.queue = new Promise<void>(resolve => { resume = resolve; });
  const old = assert.rejects(h.handlers.get('completion')({ kind: 'completion', document: 1n, version: 0n, request: 8n, cursor: 3, context: {}, snapshot: { text: () => 'fn main() {}' } }), /no longer current/);
  await h.settle();
  h.reference.version = 1n; h.content.value = 'fn changed() {}';
  await h.message({ tag: 'document-changed', val: { document: 1n, lifecycle: 1n } });
  resume(); await old; await doc.queue;
  assert.equal(doc.text.value, h.content.value);
  assert.equal(doc.reference.version, 1n);
  assert.equal(h.sent.filter(item => item.method === 'textDocument/completion').length, 0);
  await h.message({ tag: 'shutdown' });
});

test('filtered completion lists retain original resolve indices and broaden on an empty query', async () => {
  const h = harness();
  await h.message({ tag: 'document-opened', val: h.reference });
  await h.client.documents.get(1n)!.queue;
  [...h.client.sessions.values()][0]!.capabilities.completionProvider = { resolveProvider: true };
  h.responses.set('textDocument/completion', { items: [{ label: 'other' }, { label: 'HashMap', data: 42 }] });
  const call = { document: 1n, version: 0n, request: 7n, cursor: 3, snapshot: { text: () => h.content.value } };
  const filtered = await h.handlers.get('completion')({ ...call, kind: 'completion', context: { query: 'hm' } });
  assert.deepEqual(filtered.items.map((item: any) => item.label), ['HashMap']);
  assert.equal(filtered.incomplete, true);
  h.responses.set('completionItem/resolve', { documentation: 'resolved' });
  await h.handlers.get('completion-resolve')({ ...call, kind: 'completion-resolve', context: {}, item: filtered.items[0] });
  assert.equal(h.sent.find(item => item.method === 'completionItem/resolve').params.data, 42);
  const broad = await h.handlers.get('completion')({ ...call, kind: 'completion', context: { query: '' } });
  assert.deepEqual(broad.items.map((item: any) => item.label), ['other', 'HashMap']);
  assert.equal(broad.incomplete, false);
  await h.message({ tag: 'shutdown' });
});

test('watch registrations filter kinds, recover rescans and roll back partial failure', async () => {
  const h = harness();
  await h.message({ tag: 'document-opened', val: h.reference });
  await h.client.documents.get(1n)!.queue;
  const session = [...h.client.sessions.values()][0]!;
  const receive = (session as any).receive.bind(session);
  await receive('client/registerCapability', { registrations: [{ id: 'watch', method: 'workspace/didChangeWatchedFiles', registerOptions: { watchers: [{ globPattern: '/work/**/*.rs', kind: 1 }] } }] });
  assert.deepEqual(h.watches[0].request.patterns, ['**/*.rs']);
  h.watches[0].listener({ changes: [{ path: 'a.rs', kind: 'change' }, { path: 'b.rs', kind: 'create' }] });
  await h.settle();
  const change = h.sent.find(item => item.method === 'workspace/didChangeWatchedFiles');
  assert.deepEqual(change.params.changes, [{ uri: 'file:///work/b.rs', type: 1 }]);
  await assert.rejects(receive('client/registerCapability', { registrations: [{ id: 'bad', method: 'workspace/didChangeWatchedFiles', registerOptions: { watchers: [{ globPattern: '**/*.rs' }, { globPattern: '/other/**/*.rs' }] } }] }), /outside/);
  assert.equal(h.watches[1].disposed, true);
  assert.equal(session.watches.has('bad'), false);
  await assert.rejects(receive('client/registerCapability', { registrations: [
    { id: 'temporary', method: 'textDocument/hover', registerOptions: {} },
    { id: 'batch-failure', method: 'workspace/didChangeWatchedFiles', registerOptions: { watchers: [{ globPattern: '/outside/**/*.rs' }] } },
  ] }), /outside/);
  assert.equal(session.dynamic.has('temporary'), false, 'a failed registration batch must roll back its own prior entries');
  assert.equal(session.watches.has('watch'), true, 'an earlier successful registration survives');
  h.watches[0].listener({ changes: [{ kind: 'rescan' }] });
  await h.settle();
  await h.client.documents.get(1n)!.queue;
  assert.equal(h.processes.length, 2);
  assert.equal(h.watches[0].disposed, true);
  assert.equal(h.sent.filter(item => item.method === 'textDocument/didOpen').length, 2);
  await h.message({ tag: 'shutdown' });
});

test('one and eight MiB Unicode documents synchronize incrementally and reject stale lifecycle diagnostics', async () => {
  for (const size of [1024 * 1024, 8 * 1024 * 1024]) {
    const h = harness();
    const prefix = '// 中😀\r\n';
    h.content.value = prefix + 'x'.repeat(size - new TextEncoder().encode(prefix).length);
    await h.message({ tag: 'document-opened', val: { ...h.reference } });
    await h.client.documents.get(1n)!.queue;
    const session = [...h.client.sessions.values()][0]!;
    const first = h.sent.find(item => item.method === 'textDocument/didOpen');
    assert.equal(new TextEncoder().encode(first.params.textDocument.text).length, size);
    h.content.value = h.content.value.slice(0, -1) + '中😀'; h.reference.version++;
    await h.message({ tag: 'document-changed', val: { document: 1n, lifecycle: 1n } });
    await h.client.documents.get(1n)!.queue;
    const changed = h.sent.find(item => item.method === 'textDocument/didChange');
    assert.equal(changed.params.contentChanges.length, 1);
    assert.equal(changed.params.contentChanges[0].text, '中😀');
    assert.ok(changed.params.contentChanges[0].range);
    await (session as any).receive('textDocument/publishDiagnostics', { uri: 'file:///work/a.rs', version: first.params.textDocument.version, diagnostics: [{ message: 'obsolete' }] });
    assert.deepEqual(h.client.documents.get(1n)!.diagnostics, []);
    await h.message({ tag: 'document-closed', val: h.reference });
    h.reference.lifecycle++; h.reference.version = 0n;
    await h.message({ tag: 'document-opened', val: { ...h.reference } });
    await h.client.documents.get(1n)!.queue;
    const opened = h.sent.filter(item => item.method === 'textDocument/didOpen').at(-1);
    assert.ok(opened.params.textDocument.version > changed.params.textDocument.version);
    await (session as any).receive('textDocument/publishDiagnostics', { uri: 'file:///work/a.rs', version: changed.params.textDocument.version, diagnostics: [{ message: 'prior lifetime' }] });
    assert.deepEqual(h.client.documents.get(1n)!.diagnostics, []);
    await h.message({ tag: 'shutdown' });
  }
});

test('dynamic registrations combine triggers and respect document selectors', async () => {
  const h = harness();
  await h.message({ tag: 'document-opened', val: h.reference });
  await h.client.documents.get(1n)!.queue;
  const session = [...h.client.sessions.values()][0]!;
  session.dynamic.set('rust', { field: 'completionProvider', options: { documentSelector: [{ language: 'rust', pattern: '**/*.rs' }], triggerCharacters: [':'] } });
  session.dynamic.set('go', { field: 'completionProvider', options: { documentSelector: [{ language: 'go' }], triggerCharacters: ['.'] } });
  assert.deepEqual(session.effective().completionProvider.triggerCharacters, [':', '.']);
  assert.deepEqual(session.effective(h.reference).completionProvider.triggerCharacters, [':']);
  session.dynamic.delete('rust');
  assert.equal(session.effective(h.reference).completionProvider.triggerCharacters, undefined);
  await h.message({ tag: 'shutdown' });
});

test('same-ID registrations advance capability identity and failed replacements restore the prior declaration', async () => {
  const h = harness();
  await h.message({ tag: 'document-opened', val: h.reference });
  await h.client.documents.get(1n)!.queue;
  const session = [...h.client.sessions.values()][0]!;
  const receive = (method: string, params: any) => (session as any).receive(method, params);
  const registration = (language: string) => ({ id: 'actions', method: 'textDocument/codeAction', registerOptions: { resolveProvider: true, documentSelector: [{ language }] } });
  await receive('client/registerCapability', { registrations: [registration('rust')] });
  const before = h.configured.get('example.rust.code-actions').revision;
  await receive('client/registerCapability', { registrations: [registration('go')] });
  assert.ok(h.configured.get('example.rust.code-actions').revision > before);
  assert.equal(session.effective(h.reference).codeActionProvider, undefined);
  const retained = session.dynamic.get('actions');
  await assert.rejects(receive('client/registerCapability', { registrations: [registration('rust'), { id: 'bad', method: 'workspace/didChangeWatchedFiles', registerOptions: { watchers: [{ globPattern: '/outside/**' }] } }] }), /outside/);
  assert.equal(session.dynamic.get('actions'), retained);
  await h.message({ tag: 'shutdown' });
});

test('workspace symbols with unresolved ranges retain the baseline file-start destination', async () => {
  const h = harness();
  h.client.sessions.set('only', { stopped: false, ready: Promise.resolve(), unit: 'utf16', effective: () => ({ workspaceSymbolProvider: true }), rpc: { request: async () => [{ name: 'symbol', kind: 12, location: { uri: 'file:///work/target.rs' } }] } } as any);
  const result = await h.handlers.get('workspace-symbols')({ kind: 'workspace-symbols', request: 1n, context: {} });
  assert.equal(result.val[0].name, 'symbol');
  assert.deepEqual(result.val[0].location.range.start, { line: 0, column: 0 });
});

test('idle sessions shut down, reopening cancels idle stop, and initialization changes recreate the process', async () => {
  const h = harness();
  await h.message({ tag: 'document-opened', val: h.reference });
  await h.client.documents.get(1n)!.queue;
  const session = [...h.client.sessions.values()][0]!;
  await h.message({ tag: 'document-closed', val: h.reference });
  const first = session.idle!;
  assert.ok(first);
  await h.message({ tag: 'document-opened', val: h.reference });
  await h.client.documents.get(1n)!.queue;
  assert.equal(session.idle, undefined);
  assert.equal(h.processes.length, 1);
  h.settings.set('example.rust.initialization', JSON.stringify(JSON.stringify({ check: true })));
  await h.message({ tag: 'settings-changed', val: ['example.rust.initialization'] });
  await h.client.documents.get(1n)!.queue;
  assert.equal(h.processes.length, 2);
  assert.equal(h.processes[0]!.killed, 1);
  assert.deepEqual(h.sent.filter(item => item.method === 'initialize').at(-1).params.initializationOptions, { check: true });
  await h.message({ tag: 'document-closed', val: h.reference });
  [...h.client.sessions.values()][0]!.idle!.callback();
  await h.settle();
  assert.equal(h.client.sessions.size, 0);
  assert.equal(h.timers.size, 0);
});

test('semantic refresh preserves completion requests and commands keep their exact root origin', async () => {
  const h = harness();
  const calls: any[] = [];
  for (const key of ['first', 'second']) h.client.sessions.set(key, { key, stopped: false, semantic: new Map(), waitForEdits: async () => {}, effective: () => ({ completionProvider: {}, definitionProvider: true, semanticTokensProvider: { full: true }, executeCommandProvider: { commands: ['run'] } }), rpc: { async request(...args: any[]) { calls.push([key, ...args]); } } } as any);
  h.client.configure();
  const completion = h.configured.get('example.rust.completion');
  h.client.refresh(h.client.sessions.get('first')!, 'highlights');
  assert.deepEqual(h.configured.get('example.rust.completion'), completion);
  assert.equal(h.configured.get('example.rust.highlights').revision, 1n);
  assert.deepEqual(h.configured.get('example.rust.definition').modes, ['definition']);
  await h.commands.get('example.rust.completion')({ command: { name: 'run', arguments: JSON.stringify([{ $loomSession: 'second', arguments: [42] }]) } });
  assert.deepEqual(calls, [['second', 'workspace/executeCommand', { command: 'run', arguments: [42] }]]);
});

test('crashes restart after one, two and four seconds and a removed root cancels pending restart', async () => {
  const h = harness();
  await h.message({ tag: 'document-opened', val: h.reference });
  await h.client.documents.get(1n)!.queue;
  for (const delay of [1000, 2000, 4000]) {
    h.processes.at(-1)!.exit({ code: 1 }); await h.settle();
    const timer = [...h.timers].find(timer => timer.milliseconds === delay);
    assert.ok(timer, `restart after ${delay}ms`);
    timer.callback(); await h.settle();
    await h.client.documents.get(1n)!.queue;
  }
  h.processes.at(-1)!.exit({ code: 1 }); await h.settle();
  assert.equal(h.timers.size, 0, 'fourth failure stays stopped');

  const removed = harness();
  await removed.message({ tag: 'document-opened', val: removed.reference });
  await removed.client.documents.get(1n)!.queue;
  removed.processes[0]!.exit({ code: 1 }); await removed.settle();
  assert.equal(removed.timers.size, 1);
  await removed.message({ tag: 'roots-changed', val: [] });
  assert.equal(removed.timers.size, 0);
  assert.equal(removed.client.documents.size, 0);
});

test('command completion waits for earlier server edits to receive their native commit receipt', async () => {
  const h = harness();
  await h.message({ tag: 'document-opened', val: h.reference });
  await h.client.documents.get(1n)!.queue;
  const session = [...h.client.sessions.values()][0]!;
  session.capabilities.executeCommandProvider = { commands: ['run'] };
  h.responses.set('workspace/executeCommand', {});
  let commit!: (value: any) => void;
  h.api.workspace.applyWorkspaceEdit = () => new Promise(resolve => { commit = resolve; });
  const edit = (session as any).receive('workspace/applyEdit', { edit: { changes: {} } });
  let finished = false;
  const command = h.commands.get('example.rust.code-actions')({ command: { name: 'run', arguments: '[]' } }).then(() => { finished = true; });
  await h.settle();
  assert.equal(finished, false);
  commit({ tag: 'committed', val: { transaction: 1n, version: 1n } });
  assert.equal((await edit).applied, true);
  await command;
  assert.equal(finished, true);
  await h.message({ tag: 'shutdown' });
});
