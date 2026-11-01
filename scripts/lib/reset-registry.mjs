// Deliberate early-stage reset; ordinary publication never calls this module.
import path from 'node:path';
import {DIST, WORK, assert, writeAtomic} from './util.mjs';
import {buildIndex, validateIndex} from './index.mjs';
import {signIndex} from './sign.mjs';
import {definitions, readRegistry, checkSigningKey} from './release.mjs';
import {GitHubReleases} from './github.mjs';

export async function resetRegistry({config, privateKey, confirmed = false, items = definitions(),
  releases = new GitHubReleases(config.repository), registry = readRegistry,
  dist = DIST, work = WORK, now = new Date()} = {}) {
  assert(confirmed, 'Reset requires --confirm; it deletes extension releases and their tags');
  checkSigningKey(config, privateKey);
  assert(items.length && items.every(item => item.version === '0.1.0'), 'Reset requires every extension package to be version 0.1.0');
  const previous = await registry(config);
  const all = releases.list();
  assert(Array.isArray(all) && all.every(item => typeof item.tag_name === 'string'), 'Invalid release inventory');
  const refs = releases.listTags();
  assert(Array.isArray(refs) && refs.every(tag => typeof tag === 'string'), 'Invalid tag inventory');
  const tags = [...new Set([...all.map(item => item.tag_name), ...refs].filter(tag => /^loom\.(?:lang|lsp|runtime|theme|queries)\.[a-z0-9-]+-/.test(tag)))];
  const index = buildIndex({packages: [], repository: config.repository, keyId: config.index['key-id'], validityDays: config.index['validity-days'], now});
  index.sequence = (previous.maxSequence ?? previous.index.sequence) + 1;
  assert(!validateIndex(index).length, 'Invalid empty registry');
  const bytes = Buffer.from(JSON.stringify(index, null, 2) + '\n');
  const signature = signIndex(bytes, privateKey, config.index['key-id']);
  const indexFile = path.join(dist, 'index/index.json'), signatureFile = indexFile + '.sig';
  const generation = path.join(dist, 'index', 'index-' + index.sequence + '.json');
  writeAtomic(generation, bytes); writeAtomic(generation + '.sig', signature);
  releases.upload('registry', [generation, generation + '.sig']);
  let checked = await registry(config);
  assert(checked.bytes.equals(bytes) && checked.signature === signature, 'Reset generation did not verify');
  writeAtomic(indexFile, bytes); writeAtomic(signatureFile, signature);
  releases.upload('registry', [indexFile, signatureFile], {clobber: true});
  checked = await registry(config);
  assert(!checked.recovered && checked.bytes.equals(bytes) && checked.signature === signature, 'Empty current registry did not verify');
  const report = {schema: 1, sequence: index.sequence, planned: tags, deleted: []};
  const save = () => writeAtomic(path.join(work, 'release/reset.json'), JSON.stringify(report, null, 2) + '\n');
  save();
  for (const tag of tags) {
    releases.delete(tag);
    report.deleted.push(tag); save();
  }
  return report;
}
