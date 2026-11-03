import test from 'node:test';
import assert from 'node:assert/strict';
import { Rpc, Text, uri, nativePath, fileKey } from './protocol.ts';
import { completion, completionMatches, signatures, selections, workspaceEdit, semantic, semanticDelta, command } from './results.ts';

test('positions preserve Chinese, surrogate pairs, CRLF and both negotiated units', () => {
  for (const unit of ['utf8', 'utf16'] as const) {
    const text = new Text('中😀x\r\nnext', unit);
    for (const offset of [0, 3, 7, 8, 10, 11, 12, 13, 14]) assert.equal(text.offset(text.position(offset)), offset);
    assert.throws(() => text.position(4), /splits/);
    assert.throws(() => text.offset({ line: 0, character: unit === 'utf8' ? 4 : 2 }), /splits/);
  }
  for (const path of ['C:/work/中 #%.rs', '/work/a b.ts', '//server/share/中.go']) assert.equal(nativePath(uri(path)), path);
  assert.throws(() => nativePath('https://example.com/file'), /only file/);
});

test('Windows canonical paths use standard file URIs and share server document identities', () => {
  for (const [canonical, native, expected] of [
    [String.raw`\\?\C:\work\中 #%.go`, 'C:/work/中 #%.go', 'file:///C:/work/%E4%B8%AD%20%23%25.go'],
    [String.raw`\\?\UNC\server\share\中 #%.go`, '//server/share/中 #%.go', 'file://server/share/%E4%B8%AD%20%23%25.go'],
    ['//?/c:/work/a.go', 'c:/work/a.go', 'file:///c:/work/a.go'],
    ['//?/unc/server/share/a.go', '//server/share/a.go', 'file://server/share/a.go'],
  ]) {
    assert.equal(uri(canonical), expected);
    assert.equal(nativePath(uri(canonical)), native);
    assert.equal(fileKey(uri(canonical)), fileKey(uri(native)));
    assert.equal(fileKey(uri(canonical)), fileKey(uri(native.toUpperCase())));
  }
  assert.equal(uri('/work/UNC/a.go'), 'file:///work/UNC/a.go');
});

test('completion eligibility preserves native smart-case subsequences and defers Unicode', () => {
  for (const [query, label, expected] of [
    ['hm', 'HashMap', true], ['HM', 'HashMap', true], ['HM', 'hashmap', false],
    ['aa', 'a', false], ['', 'anything', true], ['中', 'ascii', true], ['x', '中文', true],
  ] as const) assert.equal(completionMatches(query, { label }), expected);
  assert.equal(completionMatches('hl', { label: 'different', filterText: 'hello' }), true);
  assert.equal(completionMatches('x', { label: 'x', filterText: '' }), false);
});

test('selection ancestors precede children in the native flat result', () => {
  const range = (start: number, end: number) => ({ start: { line: 0, character: start }, end: { line: 0, character: end } });
  assert.deepEqual(selections([{ range: range(1, 2), parent: { range: range(0, 3) } }], new Text('abc')), { tag: 'selections', val: [
    { range: { start: 0n, end: 3n }, parent: undefined },
    { range: { start: 1n, end: 2n }, parent: 0 },
  ] });
});

test('large coordinate indexes yield across surrogate boundaries without changing offsets', async () => {
  const value = 'a'.repeat(16383) + '😀中\r\n' + 'z'.repeat(140000);
  for (const unit of ['utf8', 'utf16'] as const) {
    let pauses = 0;
    const text = await Text.indexed(value, unit, async () => { pauses++; });
    assert.ok(pauses > 1);
    assert.equal(text.bytes, Buffer.byteLength(value));
    assert.deepEqual(text.position(16387), { line: 0, character: unit === 'utf8' ? 16387 : 16385 });
    assert.deepEqual(text.position(16392), { line: 1, character: 0 });
    assert.equal(text.offset({ line: 0, character: unit === 'utf8' ? 16387 : 16385 }), 16387);
    assert.throws(() => text.position(16384), /splits/);
  }
});

test('UTF-8 text transport keeps byte accounting and ignores input after closing', async () => {
  const sent: string[] = [];
  let received = 0;
  const rpc = new Rpc(async () => { throw new Error('binary path used'); }, () => ({ cancel() {} }), () => { received++; }, () => {}, undefined, async text => { sent.push(text); });
  const pending = rpc.request('test', { value: '中😀' });
  assert.equal(JSON.parse(sent[0]!).params.value, '中😀');
  const response = JSON.stringify({ jsonrpc: '2.0', id: 1, result: { value: '中😀' } });
  rpc.input(response);
  assert.equal(rpc.byteLength(await pending), Buffer.byteLength(response));
  rpc.close(); rpc.input(JSON.stringify({ jsonrpc: '2.0', method: 'late' }));
  await Promise.resolve(); assert.equal(received, 0);
});

test('million-line Unicode coordinates and large writes retain exact byte positions', async () => {
  const value = '中😀\r\n'.repeat(1000000) + 'end';
  const text = await Text.indexed(value, 'utf16', async () => {});
  assert.equal(text.bytes, 9000003);
  assert.deepEqual(text.position(9000000), { line: 1000000, character: 0 });
  assert.equal(text.offset({ line: 999999, character: 3 }), 8999998);
  assert.equal(text.offset({ line: 1000000, character: 3 }), 9000003);
  let written: Uint8Array | undefined;
  const rpc = new Rpc(async bytes => { written = bytes; }, () => ({ cancel() {} }), () => {}, () => {}, undefined, async () => { throw new Error('large write used string lowering'); });
  await rpc.notify('large', { text: value });
  assert.equal(JSON.parse(new TextDecoder().decode(written)).params.text, value);
  rpc.close();
});

test('identical completion bodies reuse read-only decoding while changed bodies and other methods decode independently', async () => {
  const rpc = new Rpc(async () => {}, () => ({ cancel() {} }), () => null, () => {});
  let id = 0;
  const reply = async (method: string, result: any) => {
    const call = rpc.request(method, {});
    rpc.input(JSON.stringify({ jsonrpc: '2.0', id: ++id, result }));
    return call;
  };
  const first = await reply('textDocument/completion', { items: [{ label: '中😀' }] });
  assert.equal(await reply('textDocument/completion', { items: [{ label: '中😀' }] }), first);
  const changed = await reply('textDocument/completion', { items: [{ label: 'different' }] });
  assert.notEqual(changed, first);
  assert.equal(changed.items[0].label, 'different');
  assert.notEqual(await reply('custom', { items: [{ label: 'different' }] }), changed);
  rpc.close();
});

test('RPC cancels only its owner, ignores late replies and fails all pending work on bad input', async () => {
  const sent: any[] = [];
  const timers: (() => void)[] = [];
  let failed = 0;
  const rpc = new Rpc(async bytes => { sent.push(JSON.parse(new TextDecoder().decode(bytes))); }, (_ms, callback) => { timers.push(callback); return { cancel() {} }; }, () => null, () => failed++);
  const first = rpc.request('one', {}, 1n).catch(error => error.message);
  const second = rpc.request('two', {}, 2n).catch(error => error.message);
  rpc.cancel(1n);
  rpc.input(new TextEncoder().encode(JSON.stringify({ jsonrpc: '2.0', id: 1, result: 'late' })));
  assert.equal(await first, 'request cancelled');
  rpc.input(new TextEncoder().encode('{broken'));
  assert.match(await second, /JSON/);
  assert.equal(failed, 1);
  assert.ok(sent.some(message => message.method === '$/cancelRequest' && message.params.id === 1));
});

test('stale queries replay once within their original deadline; commands and cancelled owners never replay', async () => {
  const sent: any[] = [], timers: (() => void)[] = [];
  const rpc = new Rpc(async bytes => { sent.push(JSON.parse(new TextDecoder().decode(bytes))); }, (_ms, callback) => { timers.push(callback); return { cancel() {} }; }, () => null, () => {});
  const input = (id: number, error: any) => rpc.input(new TextEncoder().encode(JSON.stringify({ jsonrpc: '2.0', id, error })));
  const first = rpc.request('textDocument/hover', {}, 1n, 5000, () => true).catch(error => error.message);
  input(1, { code: -32802, message: 'indexing' });
  assert.equal(timers.length, 1, 'replay must retain the original deadline');
  input(2, { code: -32802, message: 'still indexing' });
  assert.match(await first, /still indexing/);
  const command = rpc.request('workspace/executeCommand', {}, 2n).catch(error => error.message);
  input(3, { code: -32802, message: 'no replay' });
  assert.match(await command, /no replay/);
  const timed = rpc.request('textDocument/hover', {}, 3n, 5000, () => true).catch(error => error.message);
  input(4, { code: -32801, message: 'changed' }); timers.at(-1)!();
  assert.match(await timed, /timed out/);
  assert.ok(sent.some(item => item.method === '$/cancelRequest' && item.params.id === 5));
  const cancelled = rpc.request('textDocument/hover', {}, 4n, 5000, () => true).catch(error => error.message);
  rpc.cancel(4n); input(6, { code: -32802, message: 'late' });
  assert.equal(await cancelled, 'request cancelled');
  assert.equal(sent.filter(item => item.method === 'textDocument/hover').length, 5);
});

test('completion defaults, resolution data and command routing retain their source session', () => {
  const text = new Text('中😀 va');
  const raw = { label: 'value', additionalTextEdits: [{ range: { start: { line: 0, character: 0 }, end: { line: 0, character: 0 } }, newText: 'import value\n' }], command: { command: 'done', arguments: [42] } };
  const item = completion(raw, text, 'root:one', { editRange: { start: { line: 0, character: 4 }, end: { line: 0, character: 6 } }, insertTextFormat: 2, commitCharacters: ['('] });
  assert.deepEqual(item.textEdit.range, { start: 8n, end: 10n });
  assert.equal(item.insertTextFormat, 2);
  assert.deepEqual(item.commitCharacters, ['(']);
  assert.equal(JSON.parse(item.data).session, 'root:one');
  assert.equal(JSON.parse(item.data).value.insertTextFormat, 2);
  assert.deepEqual(JSON.parse(item.data).value.commitCharacters, ['(']);
  assert.deepEqual(JSON.parse(item.command.arguments), [{ $loomSession: 'root:one', arguments: [42] }]);
  assert.notDeepEqual(command(raw.command, 'root:two'), item.command);
});

test('semantic deltas reject overlaps, missing baselines and invalid lengths', () => {
  assert.deepEqual(semanticDelta([0, 1, 2, 3, 4], [{ start: 1, deleteCount: 2, data: [9] }, { start: 4, deleteCount: 1 }]), [0, 9, 3]);
  assert.throws(() => semanticDelta(undefined, []), /baseline/);
  for (const edits of [[{ start: 0, deleteCount: -1 }], [{ start: 0.5, deleteCount: 0 }], [{ start: 0, deleteCount: 3 }, { start: 2, deleteCount: 1 }]]) assert.throws(() => semanticDelta([0, 1, 2], edits), /invalid|overlapping/);
});

test('multi-file edits retain order, versions and unopened UTF-16 coordinates', () => {
  const text = new Text('');
  const range = { start: { line: 0, character: 1 }, end: { line: 0, character: 3 } };
  const edit = workspaceEdit({ documentChanges: [{ kind: 'create', uri: 'file:///work/new.ts' }, { textDocument: { uri: 'file:///work/new.ts', version: 7 }, edits: [{ range, newText: '中' }] }, { kind: 'rename', oldUri: 'file:///work/new.ts', newUri: 'file:///work/next.ts' }] }, text);
  assert.deepEqual(edit.changes.map((change: any) => change.tag), ['create', 'text', 'rename']);
  assert.equal(edit.changes[1].val.version, 7n);
  assert.equal(edit.changes[1].val.edits[0].range.unit, 'utf16');
});

test('signature label ranges and semantic token offsets use UTF-8 in the host contract', () => {
  const signature = signatures({ signatures: [{ label: '中(😀)', parameters: [{ label: [2, 4] }] }] });
  assert.deepEqual(signature.val.signatures[0].parameters[0].range, { start: 4n, end: 8n });
  assert.deepEqual(semantic([0, 1, 2, 0, 0], ['variable'], new Text('中😀x')).val[0].range, { start: 3n, end: 7n });
});
