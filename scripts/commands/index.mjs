// index [--previous file] [--key file | --unsigned]: build and sign dist/index/index.json
// from the packages in dist/packages.
import fs from 'node:fs';
import path from 'node:path';
import {DIST, assert, writeAtomic} from '../lib/util.mjs';
import {loadConfig} from '../lib/config.mjs';
import {buildIndex, indexFile, readIndex, signatureFile, validateIndex} from '../lib/index.mjs';
import {signIndex} from '../lib/sign.mjs';
import {readPackages} from './pack.mjs';
import {parseArgs} from '../lib/cli.mjs';

export const usage = 'index [--previous file] [--key file | --unsigned]    Build and sign dist/index/index.json';

/** The private key from `--key`, or from the LOOM_INDEX_KEY environment variable. */
export function readKey(options) {
  if (options.key) return fs.readFileSync(path.resolve(options.key), 'utf8');
  return process.env.LOOM_INDEX_KEY || null;
}

export async function run(argv) {
  const {options} = parseArgs(argv, {previous: 'string', key: 'string', unsigned: 'boolean', now: 'string'});
  const config = loadConfig();
  const { packages } = readPackages();
  assert(packages.length, 'No packages; run build and pack first');
  const key = readKey(options);
  assert(key || options.unsigned, 'Sign the index with --key <file> (or LOOM_INDEX_KEY), or pass --unsigned for a local one');
  const previous = options.previous ? readIndex(path.resolve(options.previous)) : null;
  assert(!options.previous || previous, `No index at ${options.previous}`);
  const index = buildIndex({
    packages, repository: config.repository, keyId: config.index['key-id'], validityDays: config.index['validity-days'],
    previous, now: options.now ? new Date(options.now) : new Date(),
  });
  const problems = validateIndex(index);
  assert(!problems.length, `The index is not valid:\n  ${problems.join('\n  ')}`);
  const bytes = `${JSON.stringify(index, null, 2)}\n`;
  writeAtomic(indexFile(DIST), bytes);
  if (key) writeAtomic(signatureFile(DIST), signIndex(Buffer.from(bytes), key, config.index['key-id']));
  else fs.rmSync(signatureFile(DIST), {force: true});
  console.log(`Index ${index.sequence}: ${index.extensions.length} extension(s), ${key ? 'signed' : 'unsigned'}.`);
}
