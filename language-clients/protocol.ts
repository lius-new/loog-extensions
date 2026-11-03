// Extension-owned JSON-RPC, bundled into each plugin, never the Loom SDK/host.
export type Json = any;
export type Timer = { cancel(): void };
export type Schedule = (milliseconds: number, callback: () => void) => Timer;
const encoder = new TextEncoder();
interface Pending {
  id: number; resolve(value: Json): void; reject(error: Error): void; timer: Timer;
  owner?: bigint; method: string; params: Json; retry?: () => boolean; retried: boolean;
  started?: bigint;
}

export class Rpc {
  private next = 1;
  private pending = new Map<number, Pending>();
  private closed = false;
  private writes = 0;
  private incoming = 0;
  private sizes = new WeakMap<object, number | string>();
  // Completion results are read-only in this private client. Keep one bounded,
  // validated wire body so repeated identical server answers need no JSON tree
  // allocation. Every request still goes to the server; changed bodies, errors
  // and other methods always follow normal decoding.
  private completionResponse?: { body: string; result: Json };
  byteLength(answer: Json): number {
    if (!answer || typeof answer !== 'object') return 0;
    const value = this.sizes.get(answer);
    if (typeof value !== 'string') return value ?? Infinity;
    const length = encoder.encode(value).length;
    this.sizes.set(answer, length);
    return length;
  }
  constructor(private write: (bytes: Uint8Array) => Promise<void>, private schedule: Schedule,
    private receive: (method: string, params: Json) => Json | Promise<Json>, private failed: (error: Error) => void,
    private timing?: { now(): bigint; record(name: string, start: bigint): void },
    private writeText?: (text: string) => Promise<void>) {}

  private send(message: Json): Promise<void> {
    if (this.closed) return Promise.reject(new Error('language server connection is closed'));
    if (this.writes >= 64) return Promise.reject(new Error('language server write budget exceeded'));
    const text = JSON.stringify(message);
    // UTF-8 arrays avoid the component engine's character-by-character string
    // lowering on large document notifications. Both paths use the same queue.
    const writeText = this.writeText && text.length <= 65536 ? this.writeText : undefined;
    const bytes = writeText ? undefined : new TextEncoder().encode(text);
    const maximum = 32 * 1024 * 1024;
    if ((bytes?.length ?? text.length) > maximum || (!bytes && text.length > maximum / 3 && new TextEncoder().encode(text).length > maximum)) return Promise.reject(new Error('language server message budget exceeded'));
    this.writes++;
    return (writeText ? writeText(text) : this.write(bytes!)).catch(error => { this.fail(error); throw error; }).finally(() => this.writes--);
  }
  notify(method: string, params?: Json): Promise<void> { return this.send({ jsonrpc: '2.0', method, params }); }

  request(method: string, params: Json, owner?: bigint, timeout = 5000, retry?: () => boolean): Promise<Json> {
    if (this.closed || this.pending.size >= 32) return Promise.reject(new Error('language server request budget exceeded or connection closed'));
    return new Promise((resolve, reject) => {
      const call: Pending = { id: this.next++, resolve, reject, owner, method, params, retry, retried: false, timer: undefined! };
      call.started = this.timing?.now();
      call.timer = this.schedule(timeout, () => {
        this.pending.delete(call.id);
        reject(new Error(`${method} timed out`));
        void this.notify('$/cancelRequest', { id: call.id }).catch(() => {});
      });
      this.transmit(call);
    });
  }
  private transmit(call: Pending): void {
    this.pending.set(call.id, call);
    void this.send({ jsonrpc: '2.0', id: call.id, method: call.method, params: call.params }).catch(error => {
      if (this.pending.get(call.id) === call) { this.pending.delete(call.id); call.timer.cancel(); call.reject(error); }
    });
  }
  cancel(owner: bigint): void {
    for (const [id, call] of this.pending) {
      if (call.owner !== owner) continue;
      this.pending.delete(id); call.timer.cancel(); call.reject(new Error('request cancelled'));
      void this.notify('$/cancelRequest', { id }).catch(() => {});
    }
  }
  input(bytes: Uint8Array | string): void {
    if (this.closed) return;
    try {
      const start = this.timing?.now();
      // Three bytes per UTF-16 code unit is a safe UTF-8 upper bound. Only
      // near-limit messages need exact counting for admission; retained results
      // compute and memoize their exact size when a consumer asks for it.
      const bound = typeof bytes === 'string' ? bytes.length * 3 : bytes.length;
      const length = bound > 32 * 1024 * 1024 && typeof bytes === 'string' ? encoder.encode(bytes).length : bound;
      if (length > 32 * 1024 * 1024) throw new Error('language server message budget exceeded');
      const text = typeof bytes === 'string' ? bytes : new TextDecoder('utf-8', { fatal: true }).decode(bytes);
      const prefix = /^\{"jsonrpc":"2\.0","id":([0-9]+),"result":/.exec(text);
      const id = prefix ? Number(prefix[1]) : undefined;
      const completion = id !== undefined && Number.isSafeInteger(id) && this.pending.get(id)?.method === 'textDocument/completion';
      const body = completion ? text.slice(prefix![0].length) : undefined;
      const previous = this.completionResponse;
      const message = completion && previous && previous.body === body
        ? { jsonrpc: '2.0', id, result: previous.result }
        : JSON.parse(text);
      if (start !== undefined) this.timing!.record('json-decode', start);
      if (!message || typeof message !== 'object' || Array.isArray(message) || message.jsonrpc !== '2.0') throw new Error('invalid JSON-RPC message');
      if (typeof message.method === 'string') {
        const requested = Object.hasOwn(message, 'id');
        if (this.incoming >= 64) throw new Error('language server incoming request budget exceeded');
        this.incoming++;
        Promise.resolve().then(() => this.receive(message.method, message.params)).then(
          result => requested ? this.send({ jsonrpc: '2.0', id: message.id, result: result ?? null }) : undefined,
          error => requested ? this.send({ jsonrpc: '2.0', id: message.id, error: { code: -32603, message: String(error?.message ?? error) } }) : undefined,
        ).catch(error => this.fail(error)).finally(() => this.incoming--);
      } else {
        const call = this.pending.get(message.id);
        if (!call) return;
        this.pending.delete(message.id);
        if ([-32801, -32802].includes(message.error?.code) && message.error.data?.retriggerRequest !== false && !call.retried && call.retry?.()) {
          call.id = this.next++; call.retried = true; this.transmit(call); return;
        }
        call.timer.cancel();
        if (call.started !== undefined) this.timing!.record(`rpc:${call.method}`, call.started);
        if (message.error) call.reject(new Error(`${message.error.code}: ${message.error.message}`));
        else if (Object.hasOwn(message, 'result')) {
          if (message.result && typeof message.result === 'object' && !this.sizes.has(message.result)) this.sizes.set(message.result, typeof bytes === 'string' ? text : bytes.length);
          if (call.method === 'textDocument/completion') this.completionResponse = body !== undefined && length <= 8 * 1024 * 1024 ? { body, result: message.result } : undefined;
          call.resolve(message.result);
        }
        else call.reject(new Error('JSON-RPC response has no result'));
      }
    } catch (error) { this.fail(error instanceof Error ? error : new Error(String(error))); }
  }
  close(reason = new Error('language server stopped')): void {
    if (this.closed) return;
    this.closed = true;
    this.completionResponse = undefined;
    for (const call of this.pending.values()) { call.timer.cancel(); call.reject(reason); }
    this.pending.clear();
  }
  private fail(error: Error): void { if (!this.closed) { this.close(error); this.failed(error); } }
}

export function normalizePath(path: string): string {
  // Windows canonical paths are filesystem paths, not UNC URI authorities.
  return path.replaceAll('\\', '/').replace(/^\/\/\?\/UNC\//i, '//').replace(/^\/\/\?\/(?=[a-z]:\/)/i, '');
}
export function uri(path: string): string {
  const normalized = normalizePath(path);
  if (normalized.startsWith('//')) {
    const [authority, ...rest] = normalized.slice(2).split('/');
    return `file://${authority}/${rest.map(encodeURIComponent).join('/')}`;
  }
  return `file://${normalized.startsWith('/') ? '' : '/'}${normalized.split('/').map((part, i) => i === 0 && /^[a-z]:$/i.test(part) ? part : encodeURIComponent(part)).join('/')}`;
}
export function nativePath(value: string): string {
  const url = /^file:\/\/([^/]*)(\/[^?#]*)(?:[?#].*)?$/i.exec(value);
  if (!url) throw new Error('only file locations are supported');
  let path = decodeURIComponent(url[2]!);
  if (url[1] && url[1] !== 'localhost') return `//${url[1]}${path}`;
  if (/^\/[a-z]:/i.test(path)) path = path.slice(1);
  return path;
}
export function fileKey(value: string): string {
  const path = nativePath(value).replaceAll('\\', '/');
  return /^[a-z]:\//i.test(path) || path.startsWith('//') ? path.toLowerCase() : path;
}

export class Text {
  private positions = new Map<number, { line: number; character: number }>();
  private offsets = new Map<string, number>();
  private ranges = new Map<string, { start: bigint; end: bigint }>();
  private lastRange?: { startLine: number; startColumn: number; endLine: number; endColumn: number; value: { start: bigint; end: bigint } };
  // Packed UTF-16/UTF-8 offsets keep million-line documents within the guest's
  // memory budget; one JS object per line multiplies that cost substantially.
  private lines = new Uint32Array(128);
  private lineCount = 1;
  private size = 0;
  get bytes(): number { return this.size; }
  constructor(readonly value: string, readonly unit: 'utf8' | 'utf16' = 'utf16', deferred = false) {
    if (!deferred) this.append(0, value.length);
  }
  private append(start: number, end: number): number {
    let byte = this.size, index = start;
    while (index < end) {
      const point = this.value.codePointAt(index)!;
      byte += point < 0x80 ? 1 : point < 0x800 ? 2 : point < 0x10000 ? 3 : 4;
      index += point > 0xffff ? 2 : 1;
      if (point === 10) {
        if (this.lineCount * 2 === this.lines.length) {
          const grown = new Uint32Array(this.lines.length * 2);
          grown.set(this.lines); this.lines = grown;
        }
        this.lines[this.lineCount * 2] = index;
        this.lines[this.lineCount * 2 + 1] = byte;
        this.lineCount++;
      }
    }
    this.size = byte; return index;
  }
  static async indexed(value: string, unit: 'utf8' | 'utf16', pause: () => Promise<void>): Promise<Text> {
    if (value.length <= 131072) return new Text(value, unit);
    const text = new Text(value, unit, true);
    let at = 0;
    while (at < value.length) { at = text.append(at, Math.min(at + 16384, value.length)); await pause(); }
    return text;
  }
  position(offset: number): { line: number; character: number } {
    if (!Number.isInteger(offset) || offset < 0 || offset > this.bytes) throw new Error('invalid byte offset');
    const cached = this.positions.get(offset); if (cached) return cached;
    let low = 0, high = this.lineCount;
    while (low + 1 < high) { const mid = (low + high) >>> 1; if (this.lines[mid * 2 + 1]! <= offset) low = mid; else high = mid; }
    const startIndex = this.lines[low * 2]!, startByte = this.lines[low * 2 + 1]!;
    if (offset === this.bytes) return { line: low, character: this.unit === 'utf8' ? offset - startByte : this.value.length - startIndex };
    let index = startIndex, bytes = startByte;
    while (bytes < offset) {
      const point = this.value.codePointAt(index)!;
      bytes += point < 0x80 ? 1 : point < 0x800 ? 2 : point < 0x10000 ? 3 : 4;
      index += point > 0xffff ? 2 : 1;
    }
    if (bytes !== offset) throw new Error('byte offset splits a character');
    const answer = { line: low, character: this.unit === 'utf8' ? offset - startByte : index - startIndex };
    if (this.positions.size < 512) this.positions.set(offset, answer);
    return answer;
  }
  offset(position: { line: number; character: number }): number {
    if (!position || !Number.isInteger(position.line) || position.line < 0 || !Number.isInteger(position.character) || position.character < 0) throw new Error('invalid text position');
    const key = `${position.line}:${position.character}`;
    const cached = this.offsets.get(key); if (cached !== undefined) return cached;
    if (position.line >= this.lineCount) return this.bytes;
    const startIndex = this.lines[position.line * 2]!, startByte = this.lines[position.line * 2 + 1]!;
    const nextIndex = position.line + 1 < this.lineCount ? this.lines[(position.line + 1) * 2]! : undefined;
    const end = nextIndex === undefined ? this.value.length : nextIndex - (this.value[nextIndex - 2] === '\r' ? 2 : 1);
    const endByte = nextIndex === undefined ? this.bytes : this.lines[(position.line + 1) * 2 + 1]! - (nextIndex - end);
    if (position.character >= (this.unit === 'utf8' ? endByte - startByte : end - startIndex)) return endByte;
    let index = startIndex, bytes = startByte, column = 0;
    while (column < position.character && index < end) {
      const point = this.value.codePointAt(index)!;
      const units = point > 0xffff ? 2 : 1;
      const size = point < 0x80 ? 1 : point < 0x800 ? 2 : point < 0x10000 ? 3 : 4;
      index += units; bytes += size; column += this.unit === 'utf8' ? size : units;
    }
    if (column > position.character) throw new Error('text position splits a character');
    if (this.offsets.size < 512) this.offsets.set(key, bytes);
    return bytes;
  }
  range(range: Json): { start: bigint; end: bigint } {
    const previous = this.lastRange;
    if (previous && previous.startLine === range.start.line && previous.startColumn === range.start.character && previous.endLine === range.end.line && previous.endColumn === range.end.character) return previous.value;
    const start = this.offset(range.start), end = this.offset(range.end);
    if (start > end) throw new Error('inverted edit range');
    const key = `${start}:${end}`;
    let answer = this.ranges.get(key);
    if (!answer) {
      answer = { start: BigInt(start), end: BigInt(end) };
      const json = Object.freeze({ start: String(start), end: String(end) });
      Object.defineProperty(answer, 'toJSON', { value: () => json });
      if (this.ranges.size < 512) this.ranges.set(key, answer);
    }
    this.lastRange = { startLine: range.start.line, startColumn: range.start.character, endLine: range.end.line, endColumn: range.end.character, value: answer };
    return answer;
  }
  edit(edit: Json): Json { return { range: this.range(edit.range), text: edit.newText }; }
  fileRange(range: Json): Json { return { start: { line: range.start.line, column: range.start.character }, end: { line: range.end.line, column: range.end.character }, unit: this.unit }; }
}
export function markdown(value: Json): string | undefined {
  if (typeof value === 'string') return value;
  if (Array.isArray(value)) return value.map(markdown).filter(Boolean).join('\n\n');
  if (value?.language) return `\`\`\`${value.language}\n${value.value}\n\`\`\``;
  return typeof value?.value === 'string' ? value.value : undefined;
}
