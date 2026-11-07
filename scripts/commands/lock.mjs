// lock: pin the extensions Loog ships (loog-ext.toml `bundled`) to the versions in the
// index, and keep a copy of that index. Loog's repository stores both: the lock says
// which packages to put in the installer, the snapshot is the index it falls back to
// when the live one cannot be fetched.
import fs from 'node:fs';
import path from 'node:path';
import {DIST, assert, hash, writeAtomic} from '../lib/util.mjs';
import {loadConfig} from '../lib/config.mjs';
import {indexFile, readIndex, signatureFile} from '../lib/index.mjs';
import {parseArgs} from '../lib/cli.mjs';

export const usage = 'lock [--out dir]    Write extensions.lock and an index snapshot for Loog\'s repository';

export async function run(argv) {
  const {options} = parseArgs(argv, {out: 'string'});
  const config = loadConfig();
  const bytes = fs.existsSync(indexFile(DIST)) ? fs.readFileSync(indexFile(DIST)) : null;
  assert(bytes, 'No index; run index first');
  assert(fs.existsSync(signatureFile(DIST)), 'The index is not signed; Loog only trusts a signed snapshot');
  const index = readIndex(indexFile(DIST));
  const extensions = config.bundled.map(id => {
    const found = index.extensions.find(extension => extension.id === id);
    assert(found, `${id} is bundled but not in the index`);
    const { version, artifacts } = found.versions[0];
    return {id, version, artifacts};
  });
  const lock = {
    schema: 1,
    index: {sequence: index.sequence, keyId: index.keyId, sha256: hash(bytes)},
    extensions,
  };
  const out = path.resolve(options.out ?? path.join(DIST, 'lock'));
  writeAtomic(path.join(out, 'extensions.lock'), `${JSON.stringify(lock, null, 2)}\n`);
  writeAtomic(path.join(out, 'index.snapshot.json'), bytes);
  writeAtomic(path.join(out, 'index.snapshot.json.sig'), fs.readFileSync(signatureFile(DIST)));
  console.log(`Locked ${extensions.length} extension(s) against index ${index.sequence} in ${out}`);
}
