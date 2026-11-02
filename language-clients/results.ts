import { Text, markdown, nativePath, type Json } from './protocol.ts';

export const opaque = (session: string, value: Json): string => JSON.stringify({ session, value });
export function original(data: string | undefined): { session: string; value: Json } | undefined {
  if (!data) return undefined;
  let result = JSON.parse(data);
  if (typeof result === 'string') result = JSON.parse(result);
  if (Array.isArray(result) && result.length === 2 && result.every(Number.isSafeInteger)) return { session: '', value: { $loomCompletion: { batch: result[0], index: result[1] } } };
  return typeof result?.session === 'string' && Object.hasOwn(result, 'value') ? result : undefined;
}
export const command = (value: Json, session: string): Json => value?.command ? { name: value.command, arguments: JSON.stringify([{ $loomSession: session, arguments: value.arguments ?? [] }]) } : undefined;
export const location = (value: Json, text: Text): Json => ({ path: nativePath(value.targetUri ?? value.uri), range: text.fileRange(value.targetSelectionRange ?? value.targetRange ?? value.range) });

export function workspaceEdit(value: Json, text: Text, version?: (url: string, value: number) => bigint): Json {
  const changes: Json[] = [];
  for (const [url, edits] of Object.entries(value?.changes ?? {})) {
    changes.push({ tag: 'text', val: { path: nativePath(url), version: undefined, edits: (edits as Json[]).map(edit => ({ range: text.fileRange(edit.range), text: edit.newText })) } });
  }
  for (const change of value?.documentChanges ?? []) {
    const options = change.options ?? {};
    if (change.kind === 'create') changes.push({ tag: 'create', val: { path: nativePath(change.uri), overwrite: !!options.overwrite, ignoreIfExists: !!options.ignoreIfExists } });
    else if (change.kind === 'rename') changes.push({ tag: 'rename', val: { source: nativePath(change.oldUri), target: nativePath(change.newUri), overwrite: !!options.overwrite, ignoreIfExists: !!options.ignoreIfExists } });
    else if (change.kind === 'delete') changes.push({ tag: 'delete', val: { path: nativePath(change.uri), recursive: !!options.recursive, ignoreIfNotExists: !!options.ignoreIfNotExists } });
    else if (change.textDocument) changes.push({ tag: 'text', val: { path: nativePath(change.textDocument.uri), version: change.textDocument.version == null ? undefined : version ? version(change.textDocument.uri, change.textDocument.version) : BigInt(change.textDocument.version), edits: change.edits.map((edit: Json) => ({ range: text.fileRange(edit.range), text: edit.newText })) } });
    else throw new Error('unsupported workspace change');
  }
  return { changes };
}

const empty = Object.freeze([]);
/** A conservative eligibility check matching the native fuzzy consumer. Unicode
 * stays with the native matcher; ASCII uses its exact smart-case subsequence rule. */
export function completionMatches(query: string | undefined, item: Json): boolean {
  return completionMatcher(query)(item);
}
export function completionMatcher(query: string | undefined): (item: Json) => boolean {
  if (!query || query.length > 256 || /[^\x00-\x7f]/.test(query)) return () => true;
  const smart = /[A-Z]/.test(query);
  const pattern = smart ? query : query.toLowerCase();
  return item => {
    const candidate = item.filterText ?? item.label;
    if (typeof candidate !== 'string') return true;
    const value = smart ? candidate : candidate.toLowerCase();
    let from = 0;
    for (let index = 0; index < pattern.length; index++) {
      const at = value.indexOf(pattern[index]!, from);
      if (at < 0) return /[^\x00-\x7f]/.test(candidate);
      from = at + 1;
    }
    return true;
  };
}

export function completion(item: Json, text: Text, session: string, defaults: Json = {}, data?: string | null): Json {
  const edit = item.textEdit ?? (defaults.editRange ? { range: defaults.editRange.replace ?? defaults.editRange, newText: item.textEditText ?? item.insertText ?? item.label } : undefined);
  return {
    label: item.label, detail: item.detail, documentation: markdown(item.documentation), kind: item.kind,
    labelDetail: item.labelDetails?.detail, labelDescription: item.labelDetails?.description,
    deprecated: item.deprecated ?? item.tags?.includes(1), preselect: item.preselect,
    insertTextMode: item.insertTextMode ?? defaults.insertTextMode,
    insertText: item.insertText, textEdit: edit ? { range: text.range(edit.replace ?? edit.range), text: edit.newText } : undefined,
    additionalTextEdits: item.additionalTextEdits?.length ? item.additionalTextEdits.map((edit: Json) => text.edit(edit)) : empty,
    insertTextFormat: item.insertTextFormat ?? defaults.insertTextFormat,
    commitCharacters: item.commitCharacters ?? defaults.commitCharacters ?? empty,
    command: command(item.command, session), sortText: item.sortText, filterText: item.filterText,
    data: data === null ? undefined : data ?? opaque(session, { ...item, textEdit: edit, insertTextFormat: item.insertTextFormat ?? defaults.insertTextFormat,
      insertTextMode: item.insertTextMode ?? defaults.insertTextMode, commitCharacters: item.commitCharacters ?? defaults.commitCharacters,
      data: item.data ?? defaults.data }),
  };
}

export function diagnostics(values: Json[], text: Text, source: string): Json[] {
  const results: Json[] = [];
  for (const value of values) {
    try { results.push({ range: text.range(value.range), severity: value.severity ?? 1, message: value.message, source: value.source ?? source, code: value.code == null ? undefined : String(value.code), documentationUrl: value.codeDescription?.href }); }
    catch { /* A malformed diagnostic cannot invalidate the other findings. */ }
  }
  return results;
}

export function symbols(values: Json[], text: Text): Json[] {
  const result: Json[] = [];
  const visit = (items: Json[], parent?: number, depth = 0) => {
    if (depth > 128) throw new Error('symbol tree depth exceeded');
    for (const item of items) {
      const range = item.range ?? item.location?.range;
      if (!range) continue;
      const index = result.length;
      result.push({ name: item.name, detail: item.detail, deprecated: item.deprecated ?? item.tags?.includes(1), kind: item.kind, range: text.range(range), selection: text.range(item.selectionRange ?? range), parent });
      visit(item.children ?? [], index, depth + 1);
    }
  };
  visit(values); return result;
}

export function action(value: Json, text: Text, session: string, version?: (url: string, value: number) => bigint): Json {
  return { title: value.title, kind: value.kind, preferred: !!value.isPreferred, disabled: value.disabled?.reason,
    edit: value.edit ? workspaceEdit(value.edit, text, version) : undefined,
    command: command(typeof value.command === 'string' ? value : value.command, session), data: opaque(session, value) };
}
export function hierarchy(value: Json, text: Text, session: string): Json {
  return { name: value.name, kind: value.kind, detail: value.detail, location: location(value, text), selection: text.fileRange(value.selectionRange), data: opaque(session, value) };
}
export function lens(value: Json, text: Text, session: string): Json {
  return { range: text.range(value.range), title: value.command?.title, command: command(value.command, session), data: opaque(session, value) };
}

export function signatures(value: Json): Json {
  const encoder = new TextEncoder();
  return { tag: 'signatures', val: { activeSignature: value?.activeSignature ?? 0, activeParameter: value?.activeParameter,
    signatures: (value?.signatures ?? []).map((item: Json) => ({ label: item.label, documentation: markdown(item.documentation), activeParameter: item.activeParameter,
      parameters: (item.parameters ?? []).map((parameter: Json) => {
        const [start, end] = Array.isArray(parameter.label) ? parameter.label : [undefined, undefined];
        return { label: start === undefined ? parameter.label : item.label.slice(start, end), documentation: markdown(parameter.documentation),
          range: start === undefined ? undefined : { start: BigInt(encoder.encode(item.label.slice(0, start)).length), end: BigInt(encoder.encode(item.label.slice(0, end)).length) } };
      }) })) } };
}

export function selections(values: Json[], text: Text): Json {
  const flat: Json[] = [];
  const visit = (value: Json, depth: number): number => {
    if (depth >= 64 || flat.length >= 256) throw new Error('selection tree budget exceeded');
    const parent = value.parent ? visit(value.parent, depth + 1) : undefined;
    const index = flat.length;
    flat.push({ range: text.range(value.range), parent });
    return index;
  };
  for (const value of values) visit(value, 0);
  return { tag: 'selections', val: flat };
}

export function semanticDelta(previous: number[] | undefined, edits: Json[]): number[] {
  if (!previous) throw new Error('semantic delta has no baseline');
  const changes = [...edits].sort((a, b) => b.start - a.start);
  let boundary = previous.length;
  for (const edit of changes) {
    if (!Number.isSafeInteger(edit.start) || !Number.isSafeInteger(edit.deleteCount) || edit.start < 0 || edit.deleteCount < 0 || edit.start + edit.deleteCount > boundary || (edit.data !== undefined && !Array.isArray(edit.data))) throw new Error('invalid or overlapping semantic delta');
    boundary = edit.start;
  }
  const data = [...previous];
  for (const edit of changes) data.splice(edit.start, edit.deleteCount, ...(edit.data ?? []));
  return data;
}

export function semantic(data: number[], legend: string[], text: Text, modifiers: string[] = []): Json {
  if (data.length % 5 || !data.every(value => Number.isInteger(value) && value >= 0 && value <= 0xffffffff)) throw new Error('malformed semantic token data');
  const result: Json[] = [];
  let line = 0, character = 0;
  const classes: Record<string, string> = { namespace: 'module', type: 'type', class: 'type', enum: 'type', interface: 'type', struct: 'type', typeParameter: 'type', parameter: 'variable.parameter', variable: 'variable', property: 'property', enumMember: 'constant', function: 'function', method: 'function.method', macro: 'function.macro', keyword: 'keyword', modifier: 'keyword', comment: 'comment', string: 'string', number: 'number', regexp: 'string.regex', operator: 'operator', decorator: 'attribute' };
  for (let i = 0; i < data.length; i += 5) {
    line += data[i]!; character = data[i]! ? data[i + 1]! : character + data[i + 1]!;
    const type = legend[data[i + 3]!] ?? '';
    const has = (name: string) => { const at = modifiers.indexOf(name); return at >= 0 && at < 32 && !!(data[i + 4]! & (1 << at)); };
    let theme = classes[type];
    if (theme === 'type' && has('defaultLibrary')) theme = 'type.builtin';
    if (type === 'function' && has('defaultLibrary')) theme = 'function.builtin';
    if (type === 'variable' && has('readonly') && has('static')) theme = 'constant';
    if (!theme) continue;
    try {
      const range = text.range({ start: { line, character }, end: { line, character: character + data[i + 2]! } });
      if (range.start < range.end && (result.length === 0 || result.at(-1).range.end <= range.start)) result.push({ range, theme });
    } catch {}
  }
  return { tag: 'highlights', val: result };
}
