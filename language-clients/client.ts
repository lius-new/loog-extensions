// Private build sources of the official plugins. Every artifact owns its own
// sessions and compiles this code into its WASM; there is no shared host client.
import { Rpc, Text, uri, nativePath, fileKey, markdown, type Json, type Timer } from './protocol.ts';
import * as result from './results.ts';

export interface Configuration {
  id: string; name: string; languages: string[]; program: string; args: string[];
  markers: string[]; outermost?: boolean; fileFallback?: boolean;
  languageId(language: string, variant?: string): string;
  initialization?: Json;
}
const kinds = ['diagnostics', 'completion', 'hover', 'definition', 'document-symbols', 'signature-help', 'formatting',
  'inlay-hints', 'code-actions', 'references', 'workspace-symbols', 'document-highlights', 'rename', 'folding-ranges',
  'selection-ranges', 'call-hierarchy', 'type-hierarchy', 'code-lens', 'format-on-type', 'execute-command', 'highlights'] as const;
const fields: Record<string, string[]> = {
  completion: ['completionProvider'], hover: ['hoverProvider'], definition: ['definitionProvider', 'declarationProvider', 'typeDefinitionProvider', 'implementationProvider'],
  'document-symbols': ['documentSymbolProvider'], 'signature-help': ['signatureHelpProvider'], formatting: ['documentFormattingProvider', 'documentRangeFormattingProvider'],
  'inlay-hints': ['inlayHintProvider'], 'code-actions': ['codeActionProvider'], references: ['referencesProvider'], 'workspace-symbols': ['workspaceSymbolProvider'],
  'document-highlights': ['documentHighlightProvider'], rename: ['renameProvider'], 'folding-ranges': ['foldingRangeProvider'], 'selection-ranges': ['selectionRangeProvider'],
  'call-hierarchy': ['callHierarchyProvider'], 'type-hierarchy': ['typeHierarchyProvider'], 'code-lens': ['codeLensProvider'], 'format-on-type': ['documentOnTypeFormattingProvider'],
  'execute-command': ['executeCommandProvider'], highlights: ['semanticTokensProvider'],
};
const tokenTypes = ['namespace', 'type', 'class', 'enum', 'interface', 'struct', 'typeParameter', 'parameter', 'variable', 'property', 'enumMember', 'event', 'function', 'method', 'macro', 'keyword', 'modifier', 'comment', 'string', 'number', 'regexp', 'operator', 'decorator'];
const tokenModifiers = ['declaration', 'definition', 'readonly', 'static', 'deprecated', 'abstract', 'async', 'modification', 'documentation', 'defaultLibrary'];
function normalizedPath(path: string): string {
  const value = path.replaceAll('\\', '/');
  return value === '/' || /^[a-z]:\/$/i.test(value) ? value : value.replace(/\/+$/, '');
}
function folderKey(path: string): string { const value = fileKey(uri(path)); return value.endsWith('/') ? value : value + '/'; }
export function capabilities(): Json {
  const dynamic = { dynamicRegistration: true };
  const textDocument: Json = Object.fromEntries(['hover', 'definition', 'declaration', 'typeDefinition', 'implementation', 'references', 'documentHighlight', 'documentSymbol', 'formatting', 'rangeFormatting', 'onTypeFormatting', 'foldingRange', 'selectionRange', 'inlayHint', 'codeLens', 'callHierarchy', 'typeHierarchy'].map(name => [name, { ...dynamic }]));
  Object.assign(textDocument, {
    synchronization: { didSave: true, willSave: false, willSaveWaitUntil: false },
    completion: { ...dynamic, contextSupport: true, completionItem: { snippetSupport: true, commitCharactersSupport: true, documentationFormat: ['markdown', 'plaintext'], labelDetailsSupport: true, deprecatedSupport: true, preselectSupport: true, tagSupport: { valueSet: [1] }, insertTextModeSupport: { valueSet: [1, 2] }, insertReplaceSupport: true, resolveSupport: { properties: ['documentation', 'detail', 'additionalTextEdits', 'command'] } }, completionList: { itemDefaults: ['commitCharacters', 'editRange', 'insertTextFormat', 'insertTextMode', 'data'] } },
    signatureHelp: { ...dynamic, contextSupport: true, signatureInformation: { documentationFormat: ['markdown', 'plaintext'], activeParameterSupport: true, parameterInformation: { labelOffsetSupport: true } } },
    rename: { ...dynamic, prepareSupport: true, prepareSupportDefaultBehavior: 1 },
    codeAction: { ...dynamic, isPreferredSupport: true, disabledSupport: true, dataSupport: true, resolveSupport: { properties: ['edit', 'command'] }, codeActionLiteralSupport: { codeActionKind: { valueSet: ['', 'quickfix', 'refactor', 'refactor.extract', 'refactor.inline', 'refactor.rewrite', 'source', 'source.organizeImports', 'source.fixAll'] } } },
    diagnostic: { ...dynamic, relatedDocumentSupport: false },
    publishDiagnostics: { versionSupport: true, relatedInformation: false },
    semanticTokens: { ...dynamic, requests: { range: true, full: { delta: true } }, tokenTypes, tokenModifiers, formats: ['relative'], overlappingTokenSupport: false, multilineTokenSupport: false, serverCancelSupport: true, augmentsSyntaxTokens: true },
  });
  for (const key of ['definition', 'declaration', 'typeDefinition', 'implementation']) textDocument[key].linkSupport = true;
  textDocument.documentSymbol.hierarchicalDocumentSymbolSupport = true;
  return { general: { positionEncodings: ['utf-8', 'utf-16'] }, textDocument,
    workspace: { applyEdit: true, configuration: true, workspaceFolders: true, workspaceEdit: { documentChanges: true, resourceOperations: ['create', 'rename', 'delete'], failureHandling: 'abort' }, didChangeWatchedFiles: { ...dynamic, relativePatternSupport: true }, didChangeConfiguration: dynamic, symbol: dynamic, executeCommand: dynamic, fileOperations: { didCreate: true, didRename: true, didDelete: true }, semanticTokens: { refreshSupport: true }, diagnostics: { refreshSupport: true }, inlayHint: { refreshSupport: true }, codeLens: { refreshSupport: true } },
    window: { workDoneProgress: true, showDocument: { support: true } } };
}

interface Document {
  reference: Json; text: Text; session: Session; synchronized?: string; identity?: string; queue: Promise<void>; diagnostics: Json[];
  dirty?: boolean; scheduled?: boolean;
  serverVersion?: number; synchronization?: Promise<void>;
  signature?: Json;
}
function mergeOptions(left: Json, right: Json): Json {
  if (!left || left === true) return right || left;
  if (!right || right === true) return left || right;
  const merged = { ...left, ...right };
  for (const key of ['triggerCharacters', 'retriggerCharacters', 'moreTriggerCharacter', 'commands'])
    if (left[key] || right[key]) merged[key] = [...new Set([...(left[key] ?? []), ...(right[key] ?? [])])];
  for (const key of ['resolveProvider', 'prepareProvider', 'range', 'full'])
    if (left[key] || right[key]) merged[key] = typeof left[key] === 'object' || typeof right[key] === 'object' ? { ...(left[key] === true ? {} : left[key]), ...(right[key] === true ? {} : right[key]) } : true;
  return merged;
}
function matches(selector: Json, document: Json, language: string): boolean {
  if (selector == null) return true;
  if (!Array.isArray(selector)) return false;
  return selector.some(filter => {
    if (filter.language && filter.language !== language) return false;
    if (filter.scheme && filter.scheme !== 'file') return false;
    if (filter.pattern) {
      // The official servers use file globs, never arbitrary regular expressions.
      const pattern = typeof filter.pattern === 'string' ? filter.pattern : filter.pattern.pattern;
      if (typeof pattern !== 'string' || pattern.length > 1024) return false;
      let expression = '';
      for (let i = 0; i < pattern.length; i++) {
        const ch = pattern[i]!;
        if (ch === '*' && pattern[i + 1] === '*') { i++; if (pattern[i + 1] === '/') { i++; expression += '(?:.*/)?'; } else expression += '.*'; }
        else if (ch === '*') expression += '[^/]*';
        else if (ch === '?') expression += '[^/]';
        else expression += ch.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
      }
      const path = document.path.replaceAll('\\', '/');
      if (!new RegExp(`^${expression}$`).test(path) && !new RegExp(`^${expression}$`).test(path.split('/').at(-1)!)) return false;
    }
    return true;
  });
}
class Session {
  process: Json;
  rpc!: Rpc;
  ready: Promise<void>;
  capabilities: Json = {};
  unit: 'utf8' | 'utf16' = 'utf16';
  stopped = false;
  watches = new Map<string, { dispose(): void }[]>();
  dynamic = new Map<string, { field: string; options: Json }>();
  progress = new Map<string, { title: string; message: string }>();
  semantic = new Map<string, { id?: string; data: number[] }>();
  private edits = new Set<Promise<Json>>();
  async waitForEdits(): Promise<void> {
    while (this.edits.size) await Promise.all([...this.edits]);
  }
  version = 0;
  idle?: Timer;
  stable?: Timer;
  attempts: number;
  readonly initialization: string;
  constructor(readonly client: Client, readonly key: string, readonly root: Json, readonly cwd: string, readonly path: string, readonly attempt = 0) {
    this.attempts = attempt;
    this.initialization = JSON.stringify(client.setting('initialization', client.config.initialization ?? {}));
    this.ready = this.start();
  }
  private async start(): Promise<void> {
    const { api, config } = this.client;
    this.client.status(`${config.name}: Starting`, 0x665522);
    this.client.enableTimings();
    try {
      this.process = api.processes.spawn({ root: this.root.id, program: config.program, args: config.args, cwd: this.cwd || undefined, env: [], framing: 'content-length-text' }, (output: Json) => {
        if (output.channel === 'stdout') this.rpc.input(output.text ?? output.bytes);
        else this.client.host.log(0, new TextDecoder().decode(output.bytes).slice(0, 4000));
      });
      this.rpc = new Rpc((bytes) => this.process.writeOwned ? this.process.writeOwned(bytes) : this.process.write(bytes), api.timers.after, (method, params) => this.receive(method, params), error => this.failed(error), this.client.measuring ? { now: () => api.clock.now(), record: (name, start) => this.client.record(name, start) } : undefined, this.process.writeText ? text => this.process.writeText(text) : undefined);
      void this.process.exited.then((exit: Json) => { if (!this.stopped) this.failed(new Error(exit.error?.message ?? `language server exited (${exit.code})`)); });
      const answer = await this.rpc.request('initialize', { processId: null, clientInfo: { name: 'Loom', version: '0.1.8' }, rootUri: uri(this.path), workspaceFolders: [{ uri: uri(this.path), name: this.root.name }], capabilities: capabilities(), initializationOptions: this.client.setting('initialization', config.initialization ?? {}) }, undefined, 30000);
      if (this.stopped) throw new Error('session stopped during initialization');
      this.capabilities = answer.capabilities ?? {};
      const encoding = this.capabilities.positionEncoding;
      if (encoding && encoding !== 'utf-8' && encoding !== 'utf-16') throw new Error(`unsupported position encoding ${encoding}`);
      this.unit = encoding === 'utf-8' ? 'utf8' : 'utf16';
      await this.rpc.notify('initialized', {});
      await this.rpc.notify('workspace/didChangeConfiguration', { settings: this.client.setting('configuration', {}) });
      this.client.configure();
      this.client.status(`${config.name}: Ready`, 0x246842);
      this.stable = api.timers.after(60000, () => { this.stable = undefined; this.attempts = 0; });
    } catch (error) { this.failed(error as Error); throw error; }
  }
  effective(document?: Json): Json {
    const values = { ...this.capabilities };
    for (const value of this.dynamic.values()) {
      if (document && !matches(value.options.documentSelector, document, this.client.config.languageId(document.language, document.variant))) continue;
      values[value.field] = mergeOptions(values[value.field], value.options);
    }
    return values;
  }
  private failed(error: Error): void {
    if (this.stopped) return;
    this.stop();
    this.client.failed(this, error);
  }
  stop(): void {
    if (this.stopped) return;
    this.stopped = true;
    this.idle?.cancel(); this.stable?.cancel();
    this.rpc?.close();
    this.process?.kill();
    for (const watches of this.watches.values()) for (const watch of watches) watch.dispose();
    this.watches.clear(); this.semantic.clear(); this.progress.clear();
  }
  async shutdown(): Promise<void> {
    if (this.stopped) return;
    try { await this.rpc.request('shutdown', null, undefined, 1000); await this.rpc.notify('exit'); } catch {}
    this.stop();
  }
  private async receive(method: string, params: Json): Promise<Json> {
    const { api, host } = this.client;
    if (method === 'textDocument/publishDiagnostics') {
      const doc = this.client.byUri(params.uri);
      if (doc?.session === this && (params.version == null || params.version === doc.serverVersion) && doc.synchronized) {
        doc.diagnostics = params.diagnostics ?? [];
        api.providers.publishDiagnostics(`${this.client.config.id}.diagnostics`, doc.reference.path, doc.reference.version, result.diagnostics(doc.diagnostics, doc.text, this.client.config.name));
      }
      return;
    }
    if (method === 'workspace/configuration') return (params.items ?? []).map((item: Json) => item.section ? item.section.split('.').reduce((value: Json, key: string) => value?.[key], this.client.setting('configuration', {})) ?? null : this.client.setting('configuration', {}));
    if (method === 'workspace/workspaceFolders') return [{ uri: uri(this.path), name: this.root.name }];
    if (method === 'workspace/applyEdit') {
      const pending = api.workspace.applyWorkspaceEdit({ title: params.label ?? this.client.config.name, edit: result.workspaceEdit(params.edit, new Text('', this.unit), this.client.protocolVersion.bind(this.client)) });
      this.edits.add(pending);
      let done: Json;
      try { done = await pending; } finally { this.edits.delete(pending); }
      return { applied: done.tag === 'committed', failureReason: done.tag === 'failed' ? done.val.message : undefined };
    }
    if (method === 'window/showDocument') {
      const span = params.selection ?? { start: { line: 0, character: 0 }, end: { line: 0, character: 0 } };
      const done = await api.workspace.showDocument({ path: nativePath(params.uri), range: new Text('', this.unit).fileRange(span) });
      return { success: done.tag === 'committed' };
    }
    if (method === 'window/workDoneProgress/create') return null;
    if (method === 'experimental/serverStatus') { host.log(params.health === 'error' ? 2 : 0, `serverStatus ${JSON.stringify(params)}`); return null; }
    if (method === '$/progress') {
      const token = JSON.stringify(params.token);
      if (params.value.kind === 'end') this.progress.delete(token);
      else {
        if (!this.progress.has(token) && this.progress.size >= 64) throw new Error('progress token budget exceeded');
        const previous = this.progress.get(token);
        this.progress.set(token, { title: String(params.value.title ?? previous?.title ?? '').slice(0, 2000), message: String(params.value.message ?? previous?.message ?? '').slice(0, 2000) });
      }
      const latest = [...this.progress.values()].at(-1);
      this.client.status(`${this.client.config.name}: ${latest ? `${latest.title} ${latest.message}`.trim() : 'Ready'}`, this.progress.size ? 0x665522 : 0x246842);
      return;
    }
    if (method === 'window/logMessage' || method === 'window/showMessage' || method === 'window/showMessageRequest') { host.log(params.type <= 2 ? 1 : 0, String(params.message)); return null; }
    if (method === 'client/registerCapability') {
      const registrations = params.registrations ?? [];
      if (!Array.isArray(registrations) || registrations.length > 64) throw new Error('invalid registration batch');
      const created = new Set<string>();
      const previousDynamic = new Map(this.dynamic), previousWatches = new Map(this.watches);
      const changed = new Set<string>();
      try { for (const registration of registrations) {
        if (typeof registration.id !== 'string' || !registration.id.length || registration.id.length > 256 || typeof registration.method !== 'string') throw new Error('invalid dynamic registration');
        if (created.has(registration.id)) throw new Error('duplicate dynamic registration in one batch');
        created.add(registration.id);
        const before = this.dynamic.get(registration.id);
        if (before) changed.add(before.field);
        this.dynamic.delete(registration.id); this.watches.delete(registration.id);
        if (registration.method === 'workspace/didChangeWatchedFiles') {
          if (this.watches.size >= 16 || (registration.registerOptions?.watchers ?? []).length > 16) throw new Error('watch registration budget exceeded');
          const watches: Json[] = [];
          try {
          for (const watcher of registration.registerOptions?.watchers ?? []) {
            if (watcher.kind !== undefined && (!Number.isInteger(watcher.kind) || watcher.kind < 1 || watcher.kind > 7)) throw new Error('invalid watch kinds');
            const pattern = watcher.globPattern;
            let relative = typeof pattern === 'string' ? pattern : pattern.pattern;
            const base = typeof pattern === 'object' ? pattern.baseUri?.uri ?? pattern.baseUri : undefined;
            const rootPath = normalizedPath(this.root.path);
            const basePath = normalizedPath(base ? nativePath(base) : this.path);
            if (fileKey(uri(basePath)) !== fileKey(uri(rootPath)) && !fileKey(uri(basePath)).startsWith(folderKey(rootPath))) throw new Error('watch base is outside this workspace');
            let prefix = basePath.slice(rootPath.length).replace(/^\//, '');
            if (typeof relative !== 'string') throw new Error('invalid watch glob');
            relative = relative.replaceAll('\\', '/');
            if (relative.startsWith('/') || /^[a-z]:\//i.test(relative)) {
              if (!fileKey(uri(relative)).startsWith(folderKey(rootPath))) throw new Error('absolute watch glob is outside this workspace');
              relative = relative.slice(rootPath.length + (rootPath.endsWith('/') ? 0 : 1)); prefix = '';
            }
            const watch = api.workspace.watchFiles({ root: this.root.id, patterns: [`${prefix ? prefix + '/' : ''}${relative}`] }, (event: Json) => {
              const changes = event.changes.flatMap((change: Json) => {
                const type = change.kind === 'create' ? 1 : change.kind === 'delete' ? 3 : 2;
                return change.kind === 'rescan' || !((watcher.kind ?? 7) & (1 << (type - 1))) ? [] : [{ uri: uri(`${this.root.path}/${change.path}`), type }];
              });
              if (changes.length) void this.rpc.notify('workspace/didChangeWatchedFiles', { changes }).catch(() => {});
              if (event.changes.some((change: Json) => change.kind === 'rescan')) void this.client.restart(this).catch(error => host.log(2, `Watch rescan: ${error}`));
            });
            watches.push(watch);
          }
          this.watches.set(registration.id, watches);
          await Promise.all(watches.map(watch => watch.ready));
          if (this.stopped) throw new Error('session stopped during watch registration');
          } catch (error) { this.watches.delete(registration.id); for (const watch of watches) watch.dispose(); throw error; }
        } else {
          const names: Record<string, string> = { formatting: 'documentFormatting', rangeFormatting: 'documentRangeFormatting', onTypeFormatting: 'documentOnTypeFormatting', diagnostic: 'diagnostic', 'semanticTokens': 'semanticTokens' };
          const name = registration.method.split('/')[1];
          const field = registration.method.startsWith('textDocument/') ? `${names[name] ?? name}Provider` : registration.method === 'workspace/executeCommand' ? 'executeCommandProvider' : undefined;
          if (this.dynamic.size >= 64) throw new Error('dynamic registration budget exceeded');
          if (field) { this.dynamic.set(registration.id, { field, options: registration.registerOptions ?? true }); changed.add(field); }
        }
      } } catch (error) {
        for (const id of created) if (this.watches.get(id) !== previousWatches.get(id)) for (const watch of this.watches.get(id) ?? []) watch.dispose();
        this.dynamic = previousDynamic; this.watches = previousWatches;
        if (this.stopped) { for (const watches of this.watches.values()) for (const watch of watches) watch.dispose(); this.watches.clear(); }
        throw error;
      }
      for (const id of created) if (this.watches.get(id) !== previousWatches.get(id)) for (const watch of previousWatches.get(id) ?? []) watch.dispose();
      if (changed.has('semanticTokensProvider')) this.semantic.clear();
      this.client.capabilitiesChanged(changed); return null;
    }
    if (method === 'client/unregisterCapability') {
      const registrations = params.unregisterations ?? params.unregistrations ?? [];
      if (!Array.isArray(registrations) || registrations.length > 64 || registrations.some(registration => typeof registration.id !== 'string')) throw new Error('invalid unregistration batch');
      const changed = new Set<string>();
      for (const registration of registrations) {
        const before = this.dynamic.get(registration.id);
        if (before) changed.add(before.field);
        for (const watch of this.watches.get(registration.id) ?? []) watch.dispose();
        this.watches.delete(registration.id); this.dynamic.delete(registration.id);
      }
      if (changed.has('semanticTokensProvider')) this.semantic.clear();
      this.client.capabilitiesChanged(changed); return null;
    }
    if (['workspace/semanticTokens/refresh', 'workspace/diagnostic/refresh', 'workspace/inlayHint/refresh', 'workspace/codeLens/refresh'].includes(method)) { this.client.refresh(this, { 'workspace/semanticTokens/refresh': 'highlights', 'workspace/diagnostic/refresh': 'diagnostics', 'workspace/inlayHint/refresh': 'inlay-hints', 'workspace/codeLens/refresh': 'code-lens' }[method]); return null; }
    if (method === '$/cancelRequest' || method.startsWith('$/')) return null;
    throw new Error(`unsupported server request: ${method}`);
  }
}

export class Client {
  private performanceEnabled?: boolean;
  get measuring(): boolean { return this.performanceEnabled ??= this.setting('performance', false) === true; }
  private measurements = new Map<string, number[]>();
  private completionSets = new Map<number, { session: Session; document: bigint; lifecycle: bigint; items: Json[]; defaults: Json; bytes: number }>();
  private nextCompletion = 0;
  sessions = new Map<string, Session>();
  documents = new Map<bigint, Document>();
  private restarts = new Map<string, Timer>();
  private revisions = new Map<string, bigint>();
  private widget: Json;
  private label: Json;
  constructor(readonly api: Json, readonly host: Json, readonly config: Configuration) {
    for (const kind of kinds) api.providers.register(`${config.id}.${kind}`, kind, (call: Json) => this.provide(call));
    for (const [base, resolve] of [['completion', 'completion-resolve'], ['code-actions', 'code-action-resolve'], ['rename', 'prepare-rename'], ['code-lens', 'code-lens-resolve']]) api.providers.register(`${config.id}.${base}`, resolve, (call: Json) => this.provide(call));
    for (const kind of kinds) api.providers.onCommand(`${config.id}.${kind}`, (call: Json) => this.execute(call.command));
    api.onMessage((message: Json) => this.message(message));
    api.workspace.onDidSave(async ({ document, snapshot }: Json) => {
      const doc = this.documents.get(document.document);
      if (!doc) return;
      await doc.session.ready; await doc.queue;
      const save = doc.session.effective(document).textDocumentSync;
      if (save?.save) await doc.session.rpc.notify('textDocument/didSave', { textDocument: { uri: uri(document.path) }, text: save.save.includeText ? snapshot.text() : undefined });
    });
  }
  setting(name: string, fallback: Json): Json {
    try { const value = this.host.getSetting(`${this.config.id}.${name}`); return value === undefined ? fallback : JSON.parse(JSON.parse(value)); }
    catch (error) { this.host.log(1, `Invalid ${name} JSON: ${error}`); return fallback; }
  }
  record(name: string, start: bigint): void {
    this.recordMilliseconds(name, Number(this.api.clock.now() - start) / 1000000);
  }
  enableTimings(): void { if (this.measuring) this.api.onTiming?.((name: string, elapsed: bigint) => this.recordMilliseconds(name, Number(elapsed) / 1000000)); }
  private recordMilliseconds(name: string, duration: number): void {
    const samples = this.measurements.get(name) ?? [];
    samples.push(duration);
    if (samples.length === 100) {
      samples.sort((a, b) => a - b);
      this.host.log(0, `language-metrics ${JSON.stringify({ name, count: 100, p50_ms: samples[49], p95_ms: samples[94] })}`);
      this.measurements.delete(name);
    } else this.measurements.set(name, samples);
  }
  status(text: string, color: number): void {
    try {
      if (!this.widget) {
        this.widget = this.api.ui.create('statusbar.extensions', this.api.ui.block({ id: 'language-status', label: this.config.name, background: color, width: 220, height: 28 }));
        this.label = this.api.ui.create(this.widget, this.api.ui.text(text, { id: 'language-status-text', foreground: 0xffffff, width: 220, height: 28 }));
      } else { this.widget.update({ background: color }); this.label.update({ text }); }
    } catch (error) { this.host.log(1, `Status widget: ${error}`); }
  }
  byUri(url: string): Document | undefined { return [...this.documents.values()].find(doc => fileKey(uri(doc.reference.path)) === fileKey(url)); }
  protocolVersion(url: string, version: number): bigint {
    const doc = this.byUri(url);
    if (!doc || doc.serverVersion !== version) throw new Error('workspace edit refers to an obsolete document version');
    return doc.reference.version;
  }
  private root(reference: Json): { root: Json; cwd: string; path: string } {
    const path = normalizedPath(reference.path);
    const roots = this.api.workspace.roots().filter((root: Json) => fileKey(uri(path)).startsWith(folderKey(normalizedPath(root.path)))).sort((a: Json, b: Json) => b.path.length - a.path.length);
    const root = roots[0];
    if (!root) throw new Error('open a trusted workspace folder for this document');
    const rootPath = normalizedPath(root.path);
    const file = path.slice(rootPath.length + (rootPath.endsWith('/') ? 0 : 1));
    let relative = file.split('/').slice(0, -1);
    const marked: string[] = [];
    while (true) {
      const directory = relative.join('/');
      for (const marker of this.config.markers) {
        try { this.host.readFile(root.id, `${directory ? directory + '/' : ''}${marker}`, 1); marked.push(directory); break; } catch {}
      }
      if (!relative.length) break;
      relative.pop();
    }
    const fallback = this.config.fileFallback ? file.split('/').slice(0, -1).join('/') : '';
    const cwd = (this.config.outermost ? marked.at(-1) : marked[0]) ?? fallback;
    return { root, cwd, path: cwd ? `${rootPath}${rootPath.endsWith('/') ? '' : '/'}${cwd}` : rootPath };
  }
  private session(reference: Json): Session {
    const { root, cwd, path } = this.root(reference);
    const key = `${root.id}:${cwd}`;
    const existing = this.sessions.get(key);
    if (existing && !existing.stopped) { existing.idle?.cancel(); existing.idle = undefined; return existing; }
    if (this.sessions.size >= 4) throw new Error('at most four language server processes may run per plugin');
    const session = new Session(this, key, root, cwd, path);
    this.sessions.set(key, session); return session;
  }
  private unused(session: Session): void {
    if (session.stopped || [...this.documents.values()].some(doc => doc.session === session)) return;
    session.idle?.cancel();
    session.idle = this.api.timers.after(60000, () => {
      session.idle = undefined;
      if ([...this.documents.values()].some(doc => doc.session === session)) return;
      void session.shutdown().finally(() => {
        if (this.sessions.get(session.key) === session) this.sessions.delete(session.key);
        this.configure();
        if (!this.sessions.size) this.status(`${this.config.name}: Stopped`, 0x555555);
      });
    });
  }
  private forgetCompletions(session?: Session, document?: bigint): void {
    for (const [id, entry] of this.completionSets) if ((!session || entry.session === session) && (document === undefined || entry.document === document)) this.completionSets.delete(id);
  }
  async restart(session: Session): Promise<void> {
    if (session.stopped || this.sessions.get(session.key) !== session) return;
    this.forgetCompletions(session); session.stop(); this.restarts.get(session.key)?.cancel(); this.restarts.delete(session.key);
    const next = new Session(this, session.key, session.root, session.cwd, session.path);
    this.sessions.set(next.key, next);
    for (const doc of this.documents.values()) if (doc.session === session) { doc.session = next; doc.synchronized = undefined; doc.diagnostics = []; this.queue(doc); }
    this.configure(); await next.ready; this.unused(next);
  }
  failed(session: Session, error: Error): void {
    if (this.sessions.get(session.key) !== session) return;
    this.sessions.delete(session.key);
    this.forgetCompletions(session);
    this.host.log(2, `${this.config.name}: ${error.message}`);
    this.status(`${this.config.name}: ${error.message}`, 0x9b3030);
    this.configure();
    if (session.attempts >= 3 || ![...this.documents.values()].some(doc => doc.session === session)) return;
    this.restarts.get(session.key)?.cancel();
    this.restarts.set(session.key, this.api.timers.after(1000 * 2 ** session.attempts, () => {
      this.restarts.delete(session.key);
      if (![...this.documents.values()].some(doc => doc.session === session)) return;
      const next = new Session(this, session.key, session.root, session.cwd, session.path, session.attempts + 1);
      this.sessions.set(next.key, next);
      for (const doc of this.documents.values()) if (doc.session === session) { doc.session = next; doc.synchronized = undefined; this.queue(doc); }
      void next.ready.catch(() => {});
    }));
  }
  configure(): void {
    const capabilities = [...this.sessions.values()].filter(session => !session.stopped).map(session => session.effective());
    for (const kind of kinds) {
      const sources = capabilities.filter(value => kind === 'diagnostics' || (fields[kind] ?? []).some(field => value[field]));
      const options = sources.map(value => (fields[kind] ?? []).map(field => value[field]).find(Boolean)).filter(Boolean);
      const triggers = options.flatMap(option => option.triggerCharacters ?? (option.firstTriggerCharacter ? [option.firstTriggerCharacter, ...(option.moreTriggerCharacter ?? [])] : []));
      const modes = kind === 'definition' ? [['definition', 'definitionProvider'], ['declaration', 'declarationProvider'], ['type-definition', 'typeDefinitionProvider'], ['implementation', 'implementationProvider']].filter(([, field]) => sources.some(source => source[field!])).map(([mode]) => mode)
        : kind === 'formatting' ? [['full', 'documentFormattingProvider'], ['range', 'documentRangeFormattingProvider']].filter(([, field]) => sources.some(source => source[field!])).map(([mode]) => mode) : undefined;
      this.api.providers.configure({ provider: `${this.config.id}.${kind}`, enabled: sources.length > 0, modes, revision: this.revisions.get(kind) ?? 0n, resolve: options.some(option => option.resolveProvider), prepare: options.some(option => option.prepareProvider), triggers: [...new Set(triggers)], retriggers: [...new Set(options.flatMap(option => option.retriggerCharacters ?? []))], commands: [...new Set(options.flatMap(option => option.commands ?? []))], aliases: kind === 'formatting' ? [this.config.name, this.config.id, this.config.id.split('.').at(-1)] : [] });
    }
  }
  capabilitiesChanged(changed: Set<string>): void {
    for (const [kind, names] of Object.entries(fields)) if (names.some(name => changed.has(name))) this.revisions.set(kind, (this.revisions.get(kind) ?? 0n) + 1n);
    this.configure();
  }
  refresh(session: Session, kind?: string): void {
    if (!kind || kind === 'highlights') session.semantic.clear();
    for (const target of kind ? [kind] : ['diagnostics', 'highlights', 'inlay-hints', 'code-lens']) this.revisions.set(target, (this.revisions.get(target) ?? 0n) + 1n);
    this.configure();
  }
  private queue(doc: Document): void {
    doc.dirty = true;
    if (doc.scheduled) return;
    doc.scheduled = true;
    doc.queue = doc.queue.catch(() => {}).then(async () => {
      await doc.session.ready;
      while (doc.dirty && this.documents.get(doc.reference.document) === doc) {
        doc.dirty = false;
        const captured = this.api.workspace.captureDocument(doc.reference.document);
        try { await this.sync(doc, captured.document, captured.snapshot.text()); }
        finally { captured.snapshot.release(); }
      }
    }).catch(error => { this.host.log(1, `Document sync: ${error}`); }).finally(() => { doc.scheduled = false; });
  }
  private sync(doc: Document, reference: Json, text: string): Promise<void> {
    const next = (doc.synchronization ?? Promise.resolve()).catch(() => {}).then(() => this.synchronize(doc, reference, text));
    doc.synchronization = next;
    return next;
  }
  private async synchronize(doc: Document, reference: Json, text: string): Promise<void> {
    const session = doc.session;
    await session.ready;
    if (session.stopped || !this.documents.has(reference.document)) throw new Error('document session is no longer active');
    if (doc.reference.lifecycle === reference.lifecycle && doc.reference.version > reference.version) throw new Error('document revision is no longer current');
    const identity = `${reference.lifecycle}:${reference.version}:${reference.language}:${reference.variant}`;
    if (doc.synchronized && doc.identity === identity && doc.text.value === text) return;
    let sliceStarted: bigint | undefined = this.api.clock?.now();
    const cooperate = async () => {
      const now: bigint | undefined = this.api.clock?.now();
      if (sliceStarted === undefined || now === undefined || now - sliceStarted >= 8000000n) {
        await new Promise<void>(resolve => this.api.timers.after(0, resolve)); sliceStarted = this.api.clock?.now();
      }
      if (session.stopped || this.documents.get(reference.document) !== doc) throw new Error('document session is no longer active');
    };
    const next = doc.text.value === text && doc.text.unit === session.unit
      ? doc.text : await Text.indexed(text, session.unit, cooperate);
    if (!doc.synchronized) {
      doc.serverVersion = ++session.version;
      await session.rpc.notify('textDocument/didOpen', { textDocument: { uri: uri(reference.path), languageId: this.config.languageId(reference.language, reference.variant), version: doc.serverVersion, text } });
    } else {
      const sync = session.effective(reference).textDocumentSync;
      const incremental = (typeof sync === 'number' ? sync : sync?.change) === 2;
      let start = 0, oldEnd = doc.text.value.length, newEnd = text.length;
      while (start < oldEnd && start < newEnd && doc.text.value.charCodeAt(start) === text.charCodeAt(start)) {
        start++; if ((start & 16383) === 0) await cooperate();
      }
      if (start > 0 && /[\uDC00-\uDFFF]/.test(doc.text.value[start] ?? text[start] ?? '')) start--;
      while (oldEnd > start && newEnd > start && doc.text.value.charCodeAt(oldEnd - 1) === text.charCodeAt(newEnd - 1)) {
        oldEnd--; newEnd--; if ((newEnd & 16383) === 0) await cooperate();
      }
      if (oldEnd < doc.text.value.length && /[\uDC00-\uDFFF]/.test(doc.text.value[oldEnd]!)) { oldEnd++; newEnd++; }
      const encoder = new TextEncoder();
      const changes = incremental ? [{ range: { start: doc.text.position(encoder.encode(doc.text.value.slice(0, start)).length), end: doc.text.position(encoder.encode(doc.text.value.slice(0, oldEnd)).length) }, text: text.slice(start, newEnd) }] : [{ text }];
      doc.serverVersion = ++session.version;
      await session.rpc.notify('textDocument/didChange', { textDocument: { uri: uri(reference.path), version: doc.serverVersion }, contentChanges: changes });
      doc.diagnostics = [];
    }
    doc.reference = reference; doc.text = next; doc.identity = identity;
    doc.synchronized = `${identity}:${doc.serverVersion}`;
  }
  private async message(message: Json): Promise<void> {
    const value = message.val;
    if (message.tag === 'document-opened' || message.tag === 'document-resync') {
      if (!this.config.languages.includes(value.language)) return;
      let doc = this.documents.get(value.document);
      if (doc && (doc.reference.lifecycle !== value.lifecycle || doc.reference.path !== value.path || doc.reference.language !== value.language || doc.reference.variant !== value.variant)) {
        await doc.session.rpc?.notify('textDocument/didClose', { textDocument: { uri: uri(doc.reference.path) } });
        this.documents.delete(value.document); this.forgetCompletions(undefined, value.document); this.unused(doc.session); doc = undefined;
      }
      if (!doc) {
        doc = { reference: value, text: new Text(''), session: this.session(value), queue: Promise.resolve(), diagnostics: [] };
        this.documents.set(value.document, doc);
      }
      this.queue(doc); return;
    }
    if (message.tag === 'document-changed') { const doc = this.documents.get(value.document); if (doc && doc.reference.lifecycle === value.lifecycle) this.queue(doc); return; }
    if (message.tag === 'document-closed') {
      const doc = this.documents.get(value.document);
      if (doc && doc.reference.lifecycle === value.lifecycle) { this.documents.delete(value.document); this.forgetCompletions(undefined, value.document); await doc.session.ready.catch(() => {}); await doc.queue; if (!doc.session.stopped) await doc.session.rpc.notify('textDocument/didClose', { textDocument: { uri: uri(doc.reference.path) } }); this.unused(doc.session); }
      return;
    }
    if (message.tag === 'provider-cancelled') { for (const session of this.sessions.values()) session.rpc?.cancel(value); return; }
    if (message.tag === 'roots-changed') {
      const roots = new Set(value.map((root: Json) => root.id));
      for (const [key, session] of this.sessions) if (!roots.has(session.root.id)) { this.forgetCompletions(session); session.stop(); this.sessions.delete(key); this.restarts.get(key)?.cancel(); this.restarts.delete(key); }
      for (const [key, timer] of this.restarts) if (!roots.has(BigInt(key.split(':')[0]!))) { timer.cancel(); this.restarts.delete(key); }
      for (const [id, doc] of this.documents) if (!roots.has(doc.session.root.id)) this.documents.delete(id);
      if (!this.sessions.size) this.status(`${this.config.name}: Stopped`, 0x555555);
      this.configure(); return;
    }
    if (message.tag === 'settings-changed') {
      for (const session of [...this.sessions.values()]) {
        if (session.initialization !== JSON.stringify(this.setting('initialization', this.config.initialization ?? {}))) {
          await this.restart(session);
        } else { await session.ready; await session.rpc.notify('workspace/didChangeConfiguration', { settings: this.setting('configuration', {}) }); }
      }
      return;
    }
    if (message.tag === 'workspace-files-changed') {
      for (const session of this.sessions.values()) for (const change of value) {
        const kind = change.tag;
        if (kind === 'rescan') { await this.restart(session); continue; }
        const paths = kind === 'renamed' ? change.val : [change.val];
        if (!paths.some((path: string) => fileKey(uri(path)).startsWith(folderKey(session.path)))) continue;
        const method = kind === 'created' ? 'workspace/didCreateFiles' : kind === 'renamed' ? 'workspace/didRenameFiles' : 'workspace/didDeleteFiles';
        await session.ready; await session.rpc.notify(method, { files: kind === 'renamed' ? [{ oldUri: uri(paths[0]), newUri: uri(paths[1]) }] : paths.map((path: string) => ({ uri: uri(path) })) });
      }
      return;
    }
    if (message.tag === 'shutdown') { this.forgetCompletions(); for (const timer of this.restarts.values()) timer.cancel(); this.restarts.clear(); await Promise.all([...this.sessions.values()].map(session => session.shutdown())); this.widget?.remove(); }
  }
  private async execute(command: Json): Promise<void> {
    let args = JSON.parse(command.arguments ?? '[]');
    if (!Array.isArray(args)) throw new Error('command arguments must be an array');
    const origin = args.length === 1 && typeof args[0]?.$loomSession === 'string' ? args[0] : undefined;
    if (origin) args = origin.arguments;
    const sessions = [...this.sessions.values()].filter(session => !session.stopped && (origin ? session.key === origin.$loomSession : (session.effective().executeCommandProvider?.commands ?? []).includes(command.name)));
    if (sessions.length !== 1) throw new Error('command origin needs an unambiguous session');
    await sessions[0]!.rpc.request('workspace/executeCommand', { command: command.name, arguments: args });
    // A server may answer the command before its preceding applyEdit request
    // receives a native commit receipt. Saving must wait for those edits.
    await sessions[0]!.waitForEdits();
  }
  private async provide(call: Json): Promise<Json> {
    if (call.kind === 'workspace-symbols') {
      const groups = await Promise.all([...this.sessions.values()].filter(session => !session.stopped).map(async session => {
        await session.ready;
        if (!session.effective().workspaceSymbolProvider) return [];
        const values = await session.rpc.request('workspace/symbol', { query: call.context?.query ?? '' }, call.request, 5000, () => !call.cancelled);
        const coordinates = new Text('', session.unit);
        return (values ?? []).filter((item: Json) => item.location?.uri).map((item: Json) => ({ name: item.name, kind: item.kind, container: item.containerName, location: result.location({ ...item.location, range: item.location.range ?? { start: { line: 0, character: 0 }, end: { line: 0, character: 0 } } }, coordinates) }));
      }));
      return { tag: 'workspace-symbols', val: groups.flat() };
    }
    if (call.kind === 'execute-command') {
      await this.execute({ name: call.context?.name, arguments: call.context?.data ?? '[]' });
      return { tag: 'completed' };
    }
    let doc = this.documents.get(call.document);
    // Capture after queued synchronization, so a provider waiting behind an
    // initial open cannot restore an old revision after the person has typed.
    if (doc) await doc.queue;
    const captured = this.api.workspace.captureDocument(call.document);
    try {
      if (captured.document.version !== call.version) throw new Error('provider snapshot is no longer current');
      if (!doc) { doc = { reference: captured.document, text: new Text(''), session: this.session(captured.document), queue: Promise.resolve(), diagnostics: [] }; this.documents.set(call.document, doc); }
      const reference = { ...captured.document, version: call.version };
      const identity = `${reference.lifecycle}:${reference.version}:${reference.language}:${reference.variant}`;
      // Query-only calls can reuse the exact immutable revision already sent to
      // the server. Save formatting/actions may carry temporary text at the
      // same document version, so those still read the supplied snapshot.
      const supplied = ['formatting', 'format-on-type', 'code-actions', 'code-action-resolve'].includes(call.kind) ? call.snapshot.text() : captured.snapshot.text();
      if (!doc.synchronized || doc.identity !== identity || doc.text.value !== supplied) {
        await this.sync(doc, reference, supplied);
      }
    } finally { captured.snapshot.release(); }
    if (call.cancelled) throw new Error('request cancelled before provider dispatch');
    const session = doc.session, text = doc.text, ctx = call.context ?? {};
    let conversion: bigint | undefined;
    const synchronized = doc.synchronized;
    const request = async (method: string, params: Json) => {
      const answer = await session.rpc.request(method, params, call.request, 5000, () => !call.cancelled && doc.synchronized === synchronized);
      if (this.measuring) conversion = this.api.clock.now();
      return answer;
    };
    const target = { textDocument: { uri: uri(doc.reference.path) } };
    const position = text.position(call.cursor ?? Number(call.range?.start ?? 0n));
    const range = { start: text.position(Number(call.range?.start ?? 0n)), end: text.position(Number(call.range?.end ?? BigInt(text.bytes))) };
    const point = { ...target, position };
    const options = { tabSize: ctx.tabSize ?? 4, insertSpaces: ctx.insertSpaces ?? true };
    const origin = result.original(call.item?.data ?? ctx.data);
    if (origin && !origin.value?.$loomCompletion && origin.session !== session.key) throw new Error('result belongs to another server session');
    let raw = origin?.value;
    if (raw?.$loomCompletion) {
      const { batch, index } = raw.$loomCompletion;
      const entry = this.completionSets.get(batch);
      if (!entry || entry.session !== session || entry.document !== call.document || entry.lifecycle !== doc.reference.lifecycle || !Number.isSafeInteger(index) || index < 0 || index >= entry.items.length) throw new Error('completion result has expired');
      const item = entry.items[index], defaults = entry.defaults;
      raw = { ...item, data: item.data ?? defaults.data, insertTextFormat: item.insertTextFormat ?? defaults.insertTextFormat,
        insertTextMode: item.insertTextMode ?? defaults.insertTextMode, commitCharacters: item.commitCharacters ?? defaults.commitCharacters,
        textEdit: item.textEdit ?? (defaults.editRange ? { range: defaults.editRange.replace ?? defaults.editRange, newText: item.textEditText ?? item.insertText ?? item.label } : undefined) };
    }
    const supported = session.effective(doc.reference);
    if (call.kind !== 'diagnostics' && !(fields[call.kind.replace(/-resolve$/, '')] ?? fields[call.kind] ?? []).some(field => supported[field])) {
      if (!['completion-resolve', 'code-action-resolve', 'code-lens-resolve', 'prepare-rename'].includes(call.kind)) throw new Error('provider is not registered for this document');
    }
    try { switch (call.kind) {
      case 'completion': {
        const answer = await request('textDocument/completion', { ...point, context: { triggerKind: ctx.triggerKind || (call.trigger ? 2 : 1), triggerCharacter: call.trigger } });
        const offered = Array.isArray(answer) ? answer : answer?.items ?? [];
        const bytes = session.rpc.byteLength(answer);
        let batch: number | undefined;
        if (supported.completionProvider?.resolveProvider && bytes <= 8 * 1024 * 1024) {
          while (this.completionSets.size && (this.completionSets.size >= 4 || [...this.completionSets.values()].reduce((sum, entry) => sum + entry.bytes, 0) + bytes > 8 * 1024 * 1024)) this.completionSets.delete(this.completionSets.keys().next().value!);
          batch = ++this.nextCompletion;
          this.completionSets.set(batch, { session, document: call.document, lifecycle: doc.reference.lifecycle, items: offered, defaults: answer?.itemDefaults ?? {}, bytes });
        }
        const items: Json[] = [];
        const matches = result.completionMatcher(ctx.query);
        let duration = 0;
        let sliceStarted: bigint | undefined = this.api.clock?.now();
        conversion = undefined;
        // Servers may offer thousands of globals. Keep all eligible candidates while
        // yielding between conversion batches. Bulk WIT submission uses bytes.
        for (let at = 0; at < offered.length; at += 64) {
          if (call.cancelled || session.stopped || doc.synchronized !== synchronized) throw new Error(`completion cancelled: caller=${call.cancelled}, stopped=${session.stopped}, revision=${synchronized} -> ${doc.synchronized}`);
          const started = this.measuring ? this.api.clock.now() : undefined;
          for (let index = at; index < Math.min(at + 64, offered.length); index++) {
            if (matches(offered[index])) items.push(result.completion(offered[index], text, session.key, answer?.itemDefaults,
              !supported.completionProvider?.resolveProvider ? null : batch === undefined ? undefined : `[${batch},${index}]`));
          }
          const now: bigint | undefined = this.api.clock?.now();
          if (started !== undefined && now !== undefined) duration += Number(now - started) / 1000000;
          if (sliceStarted === undefined || now === undefined || now - sliceStarted >= 8000000n) {
            await new Promise<void>(resolve => this.api.timers.after(0, resolve));
            sliceStarted = this.api.clock?.now();
          }
        }
        if (this.measuring) this.recordMilliseconds('convert:completion', duration);
        return { items, incomplete: !!answer?.isIncomplete || items.length < offered.length };
      }
      case 'completion-resolve': {
        const resolved = await request('completionItem/resolve', raw);
        return result.completion({ ...raw, ...resolved, label: resolved?.label || raw.label }, text, session.key);
      }
      case 'hover': { const answer = await request('textDocument/hover', point); return answer ? { contents: markdown(answer.contents) ?? '', range: answer.range ? text.range(answer.range) : undefined } : undefined; }
      case 'definition': { const methods: Record<string, string> = { declaration: 'declaration', 'type-definition': 'typeDefinition', implementation: 'implementation' }; const method = methods[ctx.mode] ?? 'definition'; const answer = await request(`textDocument/${method}`, point); return { tag: 'places', val: (Array.isArray(answer) ? answer : answer ? [answer] : []).map((item: Json) => result.location(item, text)) }; }
      case 'references': return { tag: 'places', val: (await request('textDocument/references', { ...point, context: { includeDeclaration: ctx.includeDeclaration ?? true } }) ?? []).map((item: Json) => result.location(item, text)) };
      case 'document-symbols': return result.symbols(await request('textDocument/documentSymbol', target) ?? [], text);
      case 'signature-help': {
        const active = doc.signature ? { ...doc.signature, activeSignature: ctx.activeSignature ?? doc.signature.activeSignature, activeParameter: ctx.activeParameter ?? doc.signature.activeParameter } : undefined;
        const answer = await request('textDocument/signatureHelp', { ...point, context: { triggerKind: ctx.triggerKind || 1, triggerCharacter: call.trigger, isRetrigger: !!ctx.retrigger, activeSignatureHelp: ctx.retrigger ? active : undefined } });
        doc.signature = answer; return result.signatures(answer);
      }
      case 'diagnostics': {
        if (supported.diagnosticProvider) { const answer = await request('textDocument/diagnostic', target); if (answer.kind === 'full') doc.diagnostics = answer.items ?? []; }
        return result.diagnostics(doc.diagnostics, text, this.config.name);
      }
      case 'formatting': return (await request(ctx.mode === 'range' ? 'textDocument/rangeFormatting' : 'textDocument/formatting', { ...target, range: ctx.mode === 'range' ? range : undefined, options }) ?? []).map((edit: Json) => text.edit(edit));
      case 'format-on-type': return (await request('textDocument/onTypeFormatting', { ...point, ch: call.trigger, options }) ?? []).map((edit: Json) => text.edit(edit));
      case 'document-highlights': return { tag: 'document-highlights', val: (await request('textDocument/documentHighlight', point) ?? []).map((item: Json) => ({ range: text.range(item.range), kind: item.kind ?? 1 })) };
      case 'prepare-rename': { const answer = await request('textDocument/prepareRename', point); return { tag: 'rename-target', val: answer ? { range: answer.range || answer.start ? text.range(answer.range ?? answer) : undefined, placeholder: answer.placeholder, defaultBehavior: !!answer.defaultBehavior } : undefined }; }
      case 'rename': return { tag: 'workspace-edit', val: result.workspaceEdit(await request('textDocument/rename', { ...point, newName: ctx.name }), text, this.protocolVersion.bind(this)) };
      case 'folding-ranges': return { tag: 'folds', val: (await request('textDocument/foldingRange', target) ?? []).map((item: Json) => ({ start: item.startLine, end: item.endLine, startColumn: item.startCharacter, endColumn: item.endCharacter, kind: item.kind, label: item.collapsedText })) };
      case 'selection-ranges': return result.selections(await request('textDocument/selectionRange', { ...target, positions: [position] }) ?? [], text);
      case 'inlay-hints': return (await request('textDocument/inlayHint', { ...target, range }) ?? []).map((item: Json) => ({ offset: BigInt(text.offset(item.position)), label: typeof item.label === 'string' ? item.label : item.label.map((part: Json) => part.value).join(''), kind: item.kind, paddingLeft: !!item.paddingLeft, paddingRight: !!item.paddingRight, tooltip: markdown(item.tooltip) }));
      case 'code-actions': {
        const start = Number(call.range?.start ?? 0n), end = Number(call.range?.end ?? BigInt(text.bytes));
        const diagnostics = doc.diagnostics.filter((item: Json) => { try { const span = text.range(item.range); return Number(span.start) <= end && Number(span.end) >= start; } catch { return false; } });
        return { tag: 'actions', val: (await request('textDocument/codeAction', { ...target, range, context: { diagnostics, only: ctx.actionKinds?.length ? ctx.actionKinds : undefined, triggerKind: ctx.triggerKind || 1 } }) ?? []).map((item: Json) => result.action(item, text, session.key, this.protocolVersion.bind(this))) };
      }
      case 'code-action-resolve': return { tag: 'actions', val: [result.action(await request('codeAction/resolve', raw), text, session.key, this.protocolVersion.bind(this))] };
      case 'code-lens': return { tag: 'lenses', val: (await request('textDocument/codeLens', target) ?? []).map((item: Json) => result.lens(item, text, session.key)) };
      case 'code-lens-resolve': return { tag: 'lenses', val: [result.lens(await request('codeLens/resolve', raw), text, session.key)] };
      case 'call-hierarchy': {
        if (ctx.mode === 'prepare' || !ctx.mode) return { tag: 'hierarchy', val: (await request('textDocument/prepareCallHierarchy', point) ?? []).map((item: Json) => result.hierarchy(item, text, session.key)) };
        const incoming = ctx.mode === 'incoming';
        return { tag: 'calls', val: (await request(incoming ? 'callHierarchy/incomingCalls' : 'callHierarchy/outgoingCalls', { item: raw }) ?? []).map((item: Json) => ({ item: result.hierarchy(incoming ? item.from : item.to, text, session.key), sites: (item.fromRanges ?? []).map((range: Json) => text.fileRange(range)) })) };
      }
      case 'type-hierarchy': return { tag: 'hierarchy', val: (await request(ctx.mode === 'prepare' || !ctx.mode ? 'textDocument/prepareTypeHierarchy' : ctx.mode === 'supertypes' ? 'typeHierarchy/supertypes' : 'typeHierarchy/subtypes', ctx.mode === 'prepare' || !ctx.mode ? point : { item: raw }) ?? []).map((item: Json) => result.hierarchy(item, text, session.key)) };
      case 'highlights': {
        const capability = supported.semanticTokensProvider;
        if (!capability) return { tag: 'highlights', val: [] };
        const previous = session.semantic.get(uri(doc.reference.path));
        const answer = await request(previous?.id && capability.full?.delta ? 'textDocument/semanticTokens/full/delta' : capability.full ? 'textDocument/semanticTokens/full' : 'textDocument/semanticTokens/range', { ...target, previousResultId: previous?.id, range: capability.full ? undefined : range });
        let data = answer?.data ?? [];
        if (answer?.edits) data = result.semanticDelta(previous?.data, answer.edits);
        session.semantic.set(uri(doc.reference.path), { id: answer?.resultId, data });
        return result.semantic(data, capability.legend?.tokenTypes ?? [], text, capability.legend?.tokenModifiers ?? []);
      }
      default: throw new Error(`unsupported provider ${call.kind}`);
    } } finally { if (conversion !== undefined) this.record(`convert:${call.kind}`, conversion); }
  }
}

export function activate(api: Json, host: Json, configuration: Configuration): Client { return new Client(api, host, configuration); }
