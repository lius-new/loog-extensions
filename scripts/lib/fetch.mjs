// Pinned downloads and safe archive extraction, shared by every builder. Nothing is
// downloaded unless its SHA-256 is written down first.
import fs from 'node:fs';
import path from 'node:path';
import * as tar from 'tar';
import {downloadVerified} from './download.mjs';
import {assert, fileHash, inside, relative} from './util.mjs';

/** Download `url` to `destination` if it is not already there with the right hash. */
export async function fetchPinned(url, expected, destination, {offline = false, ...options} = {}) {
  if (fs.existsSync(destination) && fileHash(destination) === expected) return destination;
  assert(!offline, `Missing or corrupt offline input: ${destination}`);
  assert(new URL(url).protocol === 'https:', `Only https downloads are allowed: ${url}`);
  return downloadVerified({url, destination, sha256: expected}, options);
}

/**
 * Extract a tar archive. Links that stay inside it become copies (also on Windows,
 * which may not allow links); everything else that is not a file or folder is refused.
 */
export async function extractArchive(archive, destination) {
  fs.mkdirSync(destination, {recursive: true});
  const links = [];
  await tar.x({file: archive, cwd: destination, strict: true, filter: (name, entry) => {
    relative(name.replace(/\/$/, ''), 'archive entry');
    assert(['File', 'Directory', 'SymbolicLink', 'Link'].includes(entry.type), `Unsupported archive entry: ${name}`);
    if (entry.linkpath) {
      assert(!path.posix.isAbsolute(entry.linkpath) && !/[\\:\0]/.test(entry.linkpath), `Unsafe archive link: ${name}`);
      const target = path.posix.normalize(path.posix.join(entry.type === 'Link' ? '' : path.posix.dirname(name), entry.linkpath));
      relative(target, 'archive link');
      links.push({name, target});
      return false;
    }
    return true;
  }});
  for (let pending = links; pending.length;) {
    const next = [];
    for (const link of pending) {
      const target = path.join(destination, link.target), output = path.join(destination, link.name);
      if (!fs.existsSync(target)) { next.push(link); continue; }
      const actual = inside(destination, link.target);
      fs.mkdirSync(path.dirname(output), {recursive: true});
      fs.cpSync(actual, output, {recursive: true, dereference: true});
    }
    assert(next.length < pending.length, 'Unresolvable archive links');
    pending = next;
  }
}

/** The one folder inside `directory`, or `directory` itself when there are several entries. */
export function oneRoot(directory) {
  const entries = fs.readdirSync(directory, {withFileTypes: true});
  return entries.length === 1 && entries[0].isDirectory() ? path.join(directory, entries[0].name) : directory;
}
