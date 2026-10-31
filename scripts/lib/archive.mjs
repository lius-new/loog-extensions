// `.loomext` packages: gzip-compressed tar with the extension id as the one root
// folder. Packing is reproducible (sorted entries, fixed times), and unpacking
// refuses anything that could write outside the destination.
import fs from 'node:fs';
import path from 'node:path';
import * as tar from 'tar';
import {assert, files, fileHash, relative} from './util.mjs';

export const MAX_ENTRIES = 4096;
export const MAX_UNPACKED = 512 * 1024 * 1024;

/** Pack the folder `directory` (named after the extension) into `output`. */
export async function createPackage(directory, output, id = path.basename(directory)) {
  const names = files(directory);
  assert(names.includes('manifest.json'), `${id}: manifest.json is missing`);
  assert(names.length <= MAX_ENTRIES, `${id}: too many files`);
  // tar needs the folder's name as its root; stage a copy under the id if it differs.
  const parent = path.dirname(directory);
  assert(path.basename(directory) === id, 'The folder must be named after the extension id');
  fs.mkdirSync(path.dirname(output), {recursive: true});
  const temporary = `${output}.partial`;
  await tar.c({file: temporary, cwd: parent, gzip: {level: 9}, portable: true, mtime: new Date(0), noDirRecurse: true},
    [id, ...names.map(name => path.posix.join(id, name))]);
  fs.renameSync(temporary, output);
  return {file: output, sha256: fileHash(output), size: fs.statSync(output).size};
}

/** Read the entries without unpacking: names, types and sizes. */
export async function listPackage(file) {
  const entries = [];
  await tar.t({file, onReadEntry: entry => entries.push({path: entry.path.replace(/\/$/, ''), type: entry.type, size: entry.size})});
  return entries;
}

/** Unpack `file` into `destination`, which becomes the folder named by the id. */
export async function extractPackage(file, destination, expectedId) {
  const entries = await listPackage(file);
  assert(entries.length && entries.length <= MAX_ENTRIES, 'The package has no entries or too many');
  const root = entries[0].path.split('/')[0];
  let total = 0;
  for (const entry of entries) {
    relative(entry.path, 'package entry');
    assert(entry.path === root || entry.path.startsWith(`${root}/`), 'The package has more than one root folder');
    assert(entry.type === 'File' || entry.type === 'Directory', `Only files and folders are allowed: ${entry.path}`);
    total += entry.size;
    assert(total <= MAX_UNPACKED, 'The package unpacks to too much data');
  }
  if (expectedId) assert(root === expectedId, `The package root is ${root}, not ${expectedId}`);
  assert(entries.some(entry => entry.path === `${root}/manifest.json`), 'The package has no manifest.json');
  fs.mkdirSync(destination, {recursive: true});
  await tar.x({file, cwd: destination, strict: true});
  return path.join(destination, root);
}
