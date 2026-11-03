// Real server integration of the extension-owned client. The WASM/native bridge
// is exercised separately in Loom; these tests do not claim P5 latency parity.
import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { spawn } from 'node:child_process';
import { performance } from 'node:perf_hooks';
import { Client, type Configuration } from './client.ts';

const repository = path.resolve(import.meta.dirname, '..');
const base = path.join(import.meta.dirname, '.work/acceptance');
// Match the canonical paths exposed by Loom after restoring a Windows workspace.
const documentPath = (value: string) => process.platform === 'win32' ? path.toNamespacedPath(value) : value;
fs.mkdirSync(base, { recursive: true });
const entries = [
  { name: 'rust', language: 'rust', file: 'src/main.rs', marker: 'Cargo.toml', project: '[workspace]\n[package]\nname="loom_acceptance"\nversion="0.1.0"\nedition="2021"\n',
    text: 'fn add(a: i32, b: i32) -> i32 { a + b }\nfn main() { let value = add(1, 2); println!("{}", value); }\n',
    program: path.join(repository, 'extensions/loom.lsp.rust-analyzer/dist/package/windows-x86_64/loom.lsp.rust-analyzer/rust-analyzer.exe'), args: [] },
  { name: 'go', language: 'go', file: 'main.go', marker: 'go.mod', project: 'module example.com/loomacceptance\n\ngo 1.26\n',
    text: 'package main\nimport "fmt"\ntype Counter struct{}\nfunc add(a int, b int) int { return a+b }\nfunc main() { value := add(1,2); fmt.Println(value) }\n', program: process.env.GOPLS ?? path.join(repository, 'extensions/loom.lsp.gopls/dist/package/windows-x86_64/loom.lsp.gopls/gopls.exe'), args: [] },
  ...['javascript', 'typescript'].map(language => ({ name: language, language, file: language === 'javascript' ? 'main.js' : 'main.ts', marker: language === 'javascript' ? 'jsconfig.json' : 'tsconfig.json', project: '{"compilerOptions":{"checkJs":true,"strict":true,"target":"ES2022"}}',
    text: language === 'javascript' ? '/** @param {number} a @param {number} b */\nfunction add(a, b) { return a + b; }\nconst value = add(1, 2);\nconsole.log(value);\n' : 'function add(a: number, b: number): number { return a + b; }\nconst value = add(1, 2);\nconsole.log(value);\n',
    program: process.execPath, args: [path.join(repository, 'extensions/loom.lsp.typescript-language-server/payload/node_modules/typescript-language-server/lib/cli.mjs'), '--stdio'] })),
];

for (const entry of entries) test(`real ${entry.name}: negotiated providers, edits, diagnostics and shutdown`, { timeout: 120000 }, async t => {
  const root = fs.mkdtempSync(path.join(base, entry.name + '-'));
  const source = path.join(root, entry.file);
  const secondRoot = path.join(root, 'second');
  const secondSource = path.join(secondRoot, entry.file);
  const project = ['javascript', 'typescript'].includes(entry.language) ? JSON.stringify({ ...JSON.parse(entry.project), files: [entry.file] }) : entry.project;
  fs.mkdirSync(path.dirname(source), { recursive: true });
  fs.mkdirSync(path.dirname(secondSource), { recursive: true });
  fs.writeFileSync(path.join(root, entry.marker), project);
  fs.writeFileSync(path.join(secondRoot, entry.marker), project);
  fs.writeFileSync(source, entry.text);
  fs.writeFileSync(secondSource, entry.text);
  let value = entry.text;
  const reference: any = { document: 1n, lifecycle: 1n, path: documentPath(source), language: entry.language, variant: entry.language, version: 0n };
  const secondReference = { ...reference, document: 2n, path: documentPath(secondSource) };
  let roots = [{ id: 1n, name: entry.name, path: documentPath(root) }];
  const handlers = new Map<string, any>();
  const configurations = new Map<string, any>();
  const logs: string[] = [];
  const trace: any[] = [];
  const statuses: any[] = [];
  const children: any[] = [];
  const timers = new Set<ReturnType<typeof setTimeout>>();
  const snapshots = (id = 1n) => ({ document: { ...(id === 2n ? secondReference : reference) }, snapshot: { text: () => id === 2n ? entry.text : value, release() {} } });
  let listener: any;
  const api: any = {
    clock: { now: () => process.hrtime.bigint() },
    timers: { after(ms: number, callback: () => void) { const id = setTimeout(() => { timers.delete(id); callback(); }, ms); timers.add(id); return { cancel() { clearTimeout(id); timers.delete(id); } }; } },
    providers: { register(id: string, kind: string, handler: any) { handlers.set(`${id}:${kind}`, handler); }, configure(value: any) { configurations.set(value.provider, value); }, onCommand() {}, publishDiagnostics(_provider: string, _path: string, _version: bigint, items: any[]) { /* raw results remain available on the client document */ } },
    ui: { block: (value: any) => value, text: (text: string, value: any) => ({ ...value, text }), create(_mount: any, description: any) { statuses.push(description); return { update(value: any) { statuses.push(value); }, remove() {} }; } },
    workspace: { roots: () => roots, captureDocument: snapshots, onDidSave() {}, watchFiles() { return { ready: Promise.resolve(), dispose() {} }; }, applyWorkspaceEdit: async () => ({ tag: 'committed' }), showDocument: async () => ({ tag: 'committed' }) },
    processes: { spawn(request: any, output: any) {
      const folder = request.root === 2n ? secondRoot : root;
      const child = spawn(entry.program, entry.args, { cwd: request.cwd ? path.join(folder, request.cwd) : folder, windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
      children.push(child);
      let buffer = Buffer.alloc(0);
      child.stdout.on('data', (bytes: Buffer) => {
        buffer = Buffer.concat([buffer, bytes]);
        if (buffer.length > 33 * 1024 * 1024) { child.kill(); throw new Error('test server frame exceeds budget'); }
        while (true) {
          const at = buffer.indexOf('\r\n\r\n');
          if (at < 0) return;
          const header = buffer.subarray(0, at).toString('ascii');
          const length = Number(/Content-Length:\s*(\d+)/i.exec(header)?.[1]);
          if (!Number.isInteger(length) || length < 0) throw new Error('invalid frame');
          if (buffer.length < at + 4 + length) return;
          const body = buffer.subarray(at + 4, at + 4 + length);
          const message = JSON.parse(body.toString('utf8'));
          if (message.method || message.result?.kind) trace.push(message);
          output({ channel: 'stdout', bytes: new Uint8Array(), text: body.toString('utf8') });
          buffer = buffer.subarray(at + 4 + length);
        }
      });
      child.stderr.on('data', (bytes: Buffer) => output({ channel: 'stderr', bytes }));
      const exited = new Promise(resolve => { child.once('exit', code => resolve({ code })); child.once('error', error => resolve({ error: { message: error.message } })); });
      return { exited, kill() { child.kill(); }, write(bytes: Uint8Array) { return new Promise<void>((resolve, reject) => { child.stdin.write(Buffer.concat([Buffer.from(`Content-Length: ${bytes.length}\r\n\r\n`), Buffer.from(bytes)]), error => error ? reject(error) : resolve()); }); } };
    } },
    onMessage(callback: any) { listener = callback; },
  };
  const host = { log(_level: number, text: string) { logs.push(text); }, getSetting: () => undefined,
    readFile(id: bigint, relative: string, max: number) { const folder = id === 2n ? secondRoot : root; const full = path.resolve(folder, relative); if (!full.startsWith(folder + path.sep)) throw new Error('outside root'); return { text: fs.readFileSync(full, 'utf8').slice(0, max) }; } };
  const config: Configuration = { id: `example.${entry.language}`, name: entry.name, languages: [entry.language], program: entry.program, args: entry.args, markers: [entry.marker], languageId: language => language };
  const client = new Client(api, host, config);
  t.after(async () => {
    fs.writeFileSync(path.join(root, 'debug.json'), JSON.stringify({ logs, trace, version: String(reference.version), cachedVersion: String(client.documents.get(1n)?.reference.version), diagnostics: client.documents.get(1n)?.diagnostics }, null, 2));
    await listener({ tag: 'shutdown' });
    for (const child of children) child.kill();
    for (const timer of timers) clearTimeout(timer);
    if (path.dirname(path.resolve(root)) !== path.resolve(base)) throw new Error('unsafe test cleanup path');
    // Do not remove a working directory while Windows is still releasing handles.
  });
  await listener({ tag: 'document-opened', val: { ...reference } });
  const document: any = client.documents.get(1n);
  await document.queue;
  assert.ok(document.synchronized, logs.join('\n'));
  const session = document.session;
  const caps = session.effective();
  const measurements: any = {};
  let request = 0n;
  const ask = async (kind: string, options: any = {}) => {
    const declaration = kind === 'prepare-rename' ? 'rename' : kind === 'completion-resolve' ? 'completion' : kind === 'code-action-resolve' ? 'code-actions' : kind === 'code-lens-resolve' ? 'code-lens' : kind;
    const handler = handlers.get(`${config.id}.${declaration}:${kind}`);
    assert.ok(handler, kind);
    const start = performance.now();
    const result = await handler({ request: ++request, kind, document: 1n, version: reference.version, snapshot: { text: () => value }, context: {}, cursor: Buffer.byteLength(value.slice(0, value.lastIndexOf('add(') + 1)), cancelled: false, ...options });
    measurements[kind] = performance.now() - start;
    return result;
  };
  let completion: any;
  const indexing = performance.now() + 30000;
  const warmStarted = performance.now();
  let stable = 0;
  while (performance.now() < indexing) {
    try { completion = await ask('completion'); }
    catch (error) {
      // Cold module loading may exceed the interactive deadline. This is an
      // explicit readiness phase, before any capability/performance assertion.
      if (!(error instanceof Error) || !(/timed out$|^-3280[12]:/.test(error.message))) throw error;
      completion = undefined;
    }
    stable = completion?.items.length ? stable + 1 : 0;
    if (stable >= 10 && performance.now() - warmStarted >= 2000) break;
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  assert.ok(completion?.items.length, `no completions: ${logs.join('\n')} ${JSON.stringify(caps)}`);
  if (caps.completionProvider?.resolveProvider) {
    const resolved = await ask('completion-resolve', { item: completion.items[0] });
    assert.equal(resolved.label, completion.items[0].label);
  }
  const hover = await ask('hover');
  assert.ok(hover?.contents, 'hover has content');
  const definition = await ask('definition');
  assert.ok(definition.val.length, 'definition has a location');
  for (const [mode, capability] of [['declaration', 'declarationProvider'], ['type-definition', 'typeDefinitionProvider'], ['implementation', 'implementationProvider']]) {
    const cursor = entry.name === 'go' && mode === 'implementation' ? Buffer.byteLength(value.slice(0, value.indexOf('func add'))) : entry.name === 'go' && mode === 'type-definition' ? Buffer.byteLength(value.slice(0, value.indexOf('Counter'))) : undefined;
    if (caps[capability!]) assert.ok(Array.isArray((await ask('definition', { context: { mode }, ...(cursor === undefined ? {} : { cursor }) })).val), mode);
  }
  if (caps.workspaceSymbolProvider) assert.ok(Array.isArray((await ask('workspace-symbols', { context: { query: 'add' } })).val));
  const symbols = await ask('document-symbols');
  assert.ok(symbols.some((symbol: any) => symbol.name === 'add'));
  const references = await ask('references');
  assert.ok(references.val.length, 'references include add');
  if (caps.signatureHelpProvider) {
    const signature = await ask('signature-help', { cursor: Buffer.byteLength(value.slice(0, value.lastIndexOf('add(') + 4)) });
    assert.ok(signature.val.signatures.length, 'signature help has an overload');
  }
  if (caps.renameProvider) {
    if (caps.renameProvider.prepareProvider) assert.equal((await ask('prepare-rename')).tag, 'rename-target');
    const rename = await ask('rename', { context: { name: 'sum' } });
    assert.ok(rename.val.changes.length, 'rename contains edits');
  }
  if (caps.documentFormattingProvider) assert.ok(Array.isArray(await ask('formatting')));
  if (caps.documentRangeFormattingProvider) assert.ok(Array.isArray(await ask('formatting', { context: { mode: 'range' } })));
  if (caps.documentOnTypeFormattingProvider) assert.ok(Array.isArray(await ask('format-on-type', { trigger: caps.documentOnTypeFormattingProvider.firstTriggerCharacter })));
  if (caps.documentHighlightProvider) assert.ok((await ask('document-highlights')).val.length);
  if (caps.foldingRangeProvider) assert.ok(Array.isArray((await ask('folding-ranges')).val));
  if (caps.selectionRangeProvider) assert.ok((await ask('selection-ranges')).val.length);
  if (caps.inlayHintProvider) assert.ok(Array.isArray(await ask('inlay-hints')));
  if (caps.codeLensProvider) {
    const lenses = (await ask('code-lens')).val;
    assert.ok(Array.isArray(lenses));
    if (caps.codeLensProvider.resolveProvider && lenses.length) assert.equal((await ask('code-lens-resolve', { context: { data: lenses[0].data } })).val.length, 1);
  }
  for (const [kind, capability, modes] of [
    ['call-hierarchy', 'callHierarchyProvider', ['incoming', 'outgoing']],
    ['type-hierarchy', 'typeHierarchyProvider', ['supertypes', 'subtypes']],
  ] as const) {
    if (!caps[capability]) continue;
    const cursor = entry.name === 'go' && kind === 'type-hierarchy' ? Buffer.byteLength(value.slice(0, value.indexOf('Counter'))) : undefined;
    const prepared = (await ask(kind, { context: { mode: 'prepare' }, ...(cursor === undefined ? {} : { cursor }) })).val;
    assert.ok(Array.isArray(prepared));
    if (prepared.length) for (const mode of modes) assert.ok(Array.isArray((await ask(kind, { context: { mode, data: prepared[0].data } })).val));
  }
  if (caps.semanticTokensProvider) {
    assert.ok(Array.isArray((await ask('highlights')).val));
    assert.ok(Array.isArray((await ask('highlights')).val), 'semantic delta or full refresh');
  }
  if (caps.codeActionProvider) {
    const actions = (await ask('code-actions')).val;
    assert.ok(Array.isArray(actions));
    if (caps.codeActionProvider.resolveProvider && actions.length) assert.equal((await ask('code-action-resolve', { context: { data: actions[0].data } })).val.length, 1);
  }
  // Real independent processes for two workspace roots, with navigation kept
  // in its originating root and workspace-symbol aggregation across both.
  roots.push({ id: 2n, name: 'second', path: documentPath(secondRoot) });
  await listener({ tag: 'roots-changed', val: roots });
  await listener({ tag: 'document-opened', val: secondReference });
  const secondDocument = client.documents.get(2n)!;
  await secondDocument.queue;
  assert.notEqual(secondDocument.session, session);
  assert.equal(secondDocument.session.root.id, 2n);
  const secondOptions = { document: 2n, version: 0n, snapshot: { text: () => entry.text }, cursor: Buffer.byteLength(entry.text.slice(0, entry.text.lastIndexOf('add(') + 1)) };
  const readySecond = performance.now() + 30000;
  const secondWarmStarted = performance.now();
  let secondStable = 0;
  let secondDefinition: any;
  do {
    try { secondDefinition = await ask('definition', secondOptions); }
    catch (error) { if (!(error instanceof Error) || !(/timed out$|^-3280[12]:/.test(error.message))) throw error; secondDefinition = undefined; }
    secondStable = secondDefinition?.val.length ? secondStable + 1 : 0;
    if (secondStable >= 10 && performance.now() - secondWarmStarted >= 2000) break;
    await new Promise(resolve => setTimeout(resolve, 100));
  } while (performance.now() < readySecond);
  assert.ok(secondDefinition?.val.some((item: any) => path.relative(secondRoot, item.path) === path.normalize(entry.file)));
  if (caps.workspaceSymbolProvider) {
    const untilSymbols = performance.now() + 30000;
    let found: any[] = [];
    do {
      try { found = (await ask('workspace-symbols', { context: { query: 'add' } })).val; }
      catch (error) { if (!(error instanceof Error) || !(/timed out$|^-3280[12]:/.test(error.message))) throw error; found = []; }
      if ([source, secondSource].every(file => found.some(item => path.relative(path.dirname(file), item.location.path) === path.basename(file)))) break;
      await new Promise(resolve => setTimeout(resolve, 100));
    } while (performance.now() < untilSymbols);
    assert.ok([source, secondSource].every(file => found.some(item => path.relative(path.dirname(file), item.location.path) === path.basename(file))), 'both roots contribute workspace symbols');
  }
  roots = roots.filter(root => root.id !== 2n);
  await listener({ tag: 'roots-changed', val: roots });
  assert.ok(!client.documents.has(2n));
  assert.ok(![...client.sessions.values()].some(session => session.root.id === 2n));
  value += '\n???\n'; reference.version++;
  await listener({ tag: 'document-changed', val: { document: 1n, lifecycle: 1n, fromVersion: 0n, toVersion: 1n, edits: 1 } });
  await document.queue;
  const deadline = performance.now() + 30000;
  while (!document.diagnostics.length && performance.now() < deadline) {
    if (caps.diagnosticProvider) await ask('diagnostics');
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  assert.ok(document.diagnostics.length, `syntax error produced no diagnostics: ${logs.join('\n')}`);
  assert.ok(statuses.some(status => String(status.text).includes('Ready')));
  fs.writeFileSync(path.join(root, 'observed.json'), JSON.stringify({ language: entry.language, server: entry.program, node: process.version, os: `${os.platform()} ${os.release()} ${os.arch()}`, capabilities: caps, measurements, multi_root: true, diagnostics: document.diagnostics.length, statuses }, null, 2));
});
