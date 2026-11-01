// reindex --key file: publish the icon, README and changelog of extensions that are
// already released, without a new version. Every extension's newest package is
// downloaded from the registry and checked against the signed hash, the three fields
// are read from it, and the index is written again with the next sequence and signed.
// Versions and packages in the index are not touched. The result goes to dist/index,
// where `lock` and the manual upload of the two files take over.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {DIST, assert, readJson, writeAtomic} from '../lib/util.mjs';
import {loadConfig} from '../lib/config.mjs';
import {extractPackage} from '../lib/archive.mjs';
import {downloadVerified} from '../lib/download.mjs';
import {indexFile, preview, signatureFile, validateIndex} from '../lib/index.mjs';
import {publicKeyOf, signIndex, verifyIndex} from '../lib/sign.mjs';
import {readRegistry} from '../lib/release.mjs';
import {parseArgs} from '../lib/cli.mjs';

export const usage = 'reindex --key file    Add icon, README and changelog to the released index entries, signed with the key';

export async function run(argv) {
  const {options} = parseArgs(argv, {key: 'string'});
  assert(options.key, 'Say which private key signs the index: --key <file>');
  const config = loadConfig();
  const keyId = config.index['key-id'];
  const privatePem = fs.readFileSync(path.resolve(options.key), 'utf8');
  assert(publicKeyOf(privatePem) === config.index.keys?.[keyId], `The key is not the one Loom trusts for ${keyId}; check loom-ext.toml index.keys`);

  const registry = await readRegistry(config);
  const work = fs.mkdtempSync(path.join(os.tmpdir(), 'loom-reindex-'));
  try {
    const extensions = [];
    for (const extension of registry.index.extensions) {
      const {version, artifacts} = extension.versions[0];
      const artifact = artifacts[0];
      const file = path.join(work, `${extension.id}.loomext`);
      await downloadVerified({url: artifact.url, destination: file, sha256: artifact.sha256});
      const directory = await extractPackage(file, path.join(work, extension.id), extension.id);
      const manifest = readJson(path.join(directory, 'manifest.json'));
      assert(manifest.version === version, `${extension.id}: the package is ${manifest.version}, the index says ${version}`);
      const {icon, readme, changelog, ...rest} = extension;
      extensions.push({...rest, ...preview(extension.id, directory, manifest)});
      console.log(`${extension.id} ${version}`);
    }
    const now = new Date();
    const index = {
      ...registry.index,
      generated: now.toISOString(),
      expires: new Date(now.getTime() + config.index['validity-days'] * 86400000).toISOString(),
      sequence: registry.maxSequence + 1,
      extensions,
    };
    const problems = validateIndex(index);
    assert(!problems.length, `The new index is invalid:\n  ${problems.join('\n  ')}`);
    const bytes = Buffer.from(`${JSON.stringify(index, null, 2)}\n`);
    assert(bytes.length <= 16 * 1024 * 1024, 'The index would exceed 16 MiB');
    const signature = signIndex(bytes, privatePem, keyId);
    assert(verifyIndex(bytes, signature, config.index.keys).ok, 'The new signature does not verify');
    writeAtomic(indexFile(DIST), bytes);
    writeAtomic(signatureFile(DIST), signature);
    // The immutable copies the publisher also keeps, named by sequence.
    writeAtomic(path.join(DIST, 'index', `index-${index.sequence}.json`), bytes);
    writeAtomic(path.join(DIST, 'index', `index-${index.sequence}.json.sig`), signature);
    console.log(`Index ${index.sequence} (${(bytes.length / 1024).toFixed(0)} KiB) written to ${path.join(DIST, 'index')}`);
  } finally {
    fs.rmSync(work, {recursive: true, force: true});
  }
}
