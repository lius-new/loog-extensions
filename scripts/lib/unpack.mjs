// Unpacking the archives upstream projects publish: zip (read in-process, no links),
// tar.gz, tar.xz (system XZ or Windows libarchive tar) and plain gzip. Entries cannot escape the
// destination and only files and folders are written.
import fs from 'node:fs';
import path from 'node:path';
import zlib from 'node:zlib';
import {spawn} from 'node:child_process';
import {pipeline} from 'node:stream/promises';
import * as tar from 'tar';
import {assert, relative} from './util.mjs';

export const FORMATS = ['zip', 'tar.gz', 'tar.xz', 'gz', 'raw'];
const MAX_EXPANDED = 4 * 1024 * 1024 * 1024;

/** A small zip reader: stored and deflated entries, zip64 sizes, no links. */
export function extractZip(file, destination) {
  const buffer = fs.readFileSync(file);
  let end = buffer.length - 22;
  while (end >= 0 && buffer.readUInt32LE(end) !== 0x06054b50) end--;
  assert(end >= 0, `Not a zip archive: ${file}`);
  let count = buffer.readUInt16LE(end + 10), offset = buffer.readUInt32LE(end + 16);
  if ((count === 0xffff || offset === 0xffffffff) && end >= 20 && buffer.readUInt32LE(end - 20) === 0x07064b50) {
    const record = Number(buffer.readBigUInt64LE(end - 20 + 8));
    assert(buffer.readUInt32LE(record) === 0x06064b50, 'Unreadable zip64 directory');
    count = Number(buffer.readBigUInt64LE(record + 32));
    offset = Number(buffer.readBigUInt64LE(record + 48));
  }
  fs.mkdirSync(destination, {recursive: true});
  let cursor = offset, total = 0;
  for (let index = 0; index < count; index++) {
    assert(buffer.readUInt32LE(cursor) === 0x02014b50, 'Corrupt zip directory');
    const flags = buffer.readUInt16LE(cursor + 8), method = buffer.readUInt16LE(cursor + 10);
    let compressed = buffer.readUInt32LE(cursor + 20), size = buffer.readUInt32LE(cursor + 24);
    const nameLength = buffer.readUInt16LE(cursor + 28), extraLength = buffer.readUInt16LE(cursor + 30), commentLength = buffer.readUInt16LE(cursor + 32);
    const attributes = buffer.readUInt32LE(cursor + 38);
    let local = buffer.readUInt32LE(cursor + 42);
    const name = buffer.toString('utf8', cursor + 46, cursor + 46 + nameLength);
    const extraStart = cursor + 46 + nameLength;
    if (size === 0xffffffff || compressed === 0xffffffff || local === 0xffffffff) {
      for (let at = extraStart; at + 4 <= extraStart + extraLength;) {
        const id = buffer.readUInt16LE(at), length = buffer.readUInt16LE(at + 2);
        if (id === 1) {
          let field = at + 4;
          if (size === 0xffffffff) { size = Number(buffer.readBigUInt64LE(field)); field += 8; }
          if (compressed === 0xffffffff) { compressed = Number(buffer.readBigUInt64LE(field)); field += 8; }
          if (local === 0xffffffff) local = Number(buffer.readBigUInt64LE(field));
        }
        at += 4 + length;
      }
    }
    cursor = extraStart + extraLength + commentLength;
    assert((flags & 1) === 0, `Encrypted zip entry: ${name}`);
    if (name.endsWith('/')) continue;
    const mode = attributes >>> 16;
    if ((mode & 0o170000) === 0o120000) continue; // links are never extracted
    relative(name, 'archive entry');
    assert(buffer.readUInt32LE(local) === 0x04034b50, `Corrupt zip entry: ${name}`);
    const start = local + 30 + buffer.readUInt16LE(local + 26) + buffer.readUInt16LE(local + 28);
    const data = buffer.subarray(start, start + compressed);
    const content = method === 0 ? data : method === 8 ? zlib.inflateRawSync(data) : null;
    assert(content && content.length === size, `Unsupported or truncated zip entry: ${name}`);
    total += size;
    assert(total <= MAX_EXPANDED, 'Archive expands beyond the allowed size');
    const output = path.join(destination, name);
    fs.mkdirSync(path.dirname(output), {recursive: true});
    fs.writeFileSync(output, content);
    if (mode & 0o111) fs.chmodSync(output, 0o755);
  }
}

const tarFilter = (name, entry) => {
  relative(name.replace(/\/$/, ''), 'archive entry');
  return entry.type === 'File' || entry.type === 'Directory';
};

export async function extractInto(archive, format, destination) {
  fs.mkdirSync(destination, {recursive: true});
  if (format === 'zip') return extractZip(archive, destination);
  if (format === 'tar.gz') return tar.x({file: archive, cwd: destination, strict: true, filter: tarFilter});
  if (format === 'tar.xz') {
    // Windows ships libarchive tar but not xz. Re-emit the archive as a tar
    // stream, then apply the same checked parser used on other platforms.
    const command = process.platform === 'win32' ? path.join(process.env.SystemRoot || 'C:/Windows', 'System32/tar.exe') : 'xz';
    const args = process.platform === 'win32' ? ['-c', '--format=pax', '-f', '-', '@' + path.resolve(archive)] : ['-dc', archive];
    const child = spawn(command, args, {windowsHide: true, stdio: ['ignore', 'pipe', 'inherit']});
    const finished = new Promise((resolve, reject) => {
      child.on('error', reject);
      child.on('close', code => (code === 0 ? resolve() : reject(new Error(`XZ conversion failed (${code}) for ${archive}`))));
    });
    try {
      await Promise.all([finished, pipeline(child.stdout, tar.x({cwd: destination, strict: true, filter: tarFilter}))]);
    } catch (error) {
      child.kill();
      throw error;
    }
    return undefined;
  }
  throw new Error(`Cannot extract ${format}`);
}
