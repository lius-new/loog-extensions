import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import {execFileSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {generateKeyPair, publicKeyOf, signIndex, verifyIndex} from '../lib/sign.mjs';
import {MAX_DOCUMENT, buildIndex, shorten, validateIndex} from '../lib/index.mjs';
import {GitHubReleases} from '../lib/github.mjs';
import {createPackage} from '../lib/archive.mjs';
import {intake} from '../lib/intake.mjs';
import {runValidator} from '../lib/validator.mjs';
import {checkout, cli, populate, write} from './cli.mjs';

const NOW = '2026-11-01T00:00:00.000Z';
const sha = bytes => createHash('sha256').update(bytes).digest('hex');
const read = (repo, file) => JSON.parse(fs.readFileSync(path.join(repo.root, file), 'utf8'));

/** Package extension `id` as some project would, at `version`, and take the package in. */
async function takeIn(repo, id, version = '1.0.0', change = () => {}) {
  const folder = path.join(repo.root, 'out', id, version);
  fs.rmSync(folder, {recursive: true, force: true});
  fs.cpSync(path.join(repo.extensions, id), folder, {recursive: true});
  const manifestFile = path.join(folder, 'manifest.json');
  fs.writeFileSync(manifestFile, JSON.stringify({...JSON.parse(fs.readFileSync(manifestFile, 'utf8')), version}));
  change(folder);
  const named = path.join(repo.root, 'out', id, `${version}-package`, id);
  fs.cpSync(folder, named, {recursive: true});
  const file = path.join(repo.root, 'handed-over', `${id}-${version}-any.loomext`);
  await createPackage(named, file, id);
  const [result] = await intake([file], {root: repo.root, dist: repo.dist, work: path.join(repo.root, '.work'), validator: null});
  assert.equal(result.error, undefined);
  return result;
}

async function packaged(t, {bundled = ['loom.theme.demo'], what = ['theme', 'language']} = {}) {
  const repo = checkout(t, {bundled}); populate(repo, what);
  execFileSync('git', ['init', '-q'], {cwd: repo.root});
  repo.dist = path.join(repo.root, 'dist');
  repo.results = [];
  for (const id of fs.readdirSync(repo.extensions)) repo.results.push(await takeIn(repo, id));
  return repo;
}

const indexOf = (results, options = {}) => buildIndex({
  packages: results.flatMap(result => result.records), repository: 'acme/extensions', keyId: 'test-1', validityDays: 30, now: new Date(NOW),
  descriptions: Object.fromEntries(results.map(result => [result.id, result.description])),
  sources: Object.fromEntries(results.map(result => [result.id, result.source])), ...options,
});

/** Write the signed index the way publication leaves it, for the lock command. */
function leaveIndex(repo, index, privateKey) {
  const bytes = Buffer.from(JSON.stringify(index, null, 2) + '\n');
  write(repo.root, 'dist/index/index.json', bytes);
  if (privateKey) write(repo.root, 'dist/index/index.json.sig', signIndex(bytes, privateKey, 'test-1'));
}

test('a signature verifies only the bytes it was made for, with the key it names', () => {
  const { privatePem, publicRaw } = generateKeyPair();
  assert.equal(publicKeyOf(privatePem), publicRaw);
  const bytes = Buffer.from('{"schema":1}\n');
  const signature = signIndex(bytes, privatePem, 'k1');
  assert.deepEqual(verifyIndex(bytes, signature, {k1: publicRaw}), {ok: true, keyId: 'k1'});
  assert.equal(verifyIndex(Buffer.from('{"schema":2}\n'), signature, {k1: publicRaw}).ok, false);
  assert.match(verifyIndex(bytes, signature, {other: publicRaw}).reason, /unknown key k1/);
  const stranger = generateKeyPair().publicRaw;
  assert.equal(verifyIndex(bytes, signature, {k1: stranger}).ok, false);
  assert.equal(verifyIndex(bytes, 'not json', {k1: publicRaw}).ok, false);
  assert.throws(() => signIndex(bytes, privatePem, 'bad id'), /Invalid key id/);
});

test('the index lists each extension with its packages and what Loom needs to choose', async t => {
  const repo = await packaged(t);
  const index = indexOf(repo.results);
  assert.deepEqual(validateIndex(index), []);
  assert.equal(index.sequence, 1);
  assert.equal(index.generated, NOW);
  assert.equal(index.expires, '2026-12-01T00:00:00.000Z');
  assert.equal(index.keyId, 'test-1');
  assert.deepEqual(index.extensions.map(extension => extension.id), ['loom.lang.demo', 'loom.theme.demo']);
  const language = index.extensions[0];
  assert.deepEqual(language.languages, [{name: 'demo', fileTypes: ['demo']}]);
  // What the list shows before installing comes along: icon, README, changelog.
  assert.equal(language.icon.format, 'svg');
  assert.match(language.icon.data, /^<svg /);
  assert.equal(language.readme, '# Test\n');
  assert.equal(typeof language.changelog, 'string');
  const version = language.versions[0];
  assert.equal(version.version, '1.0.0');
  assert.deepEqual(version.engines, {loom: '>=0.1.8 <0.3'});
  assert.deepEqual(version.artifacts.map(artifact => artifact.platform), ['any']);
  assert.equal(version.artifacts[0].url, 'https://github.com/acme/extensions/releases/download/loom.lang.demo-1.0.0/loom.lang.demo-1.0.0-any.loomext');
});

test('a changelog over the limit keeps its newest part, cut at a paragraph', () => {
  assert.equal(shorten(Buffer.from('# Changes\n')), '# Changes\n');
  const long = Buffer.from(Array.from({length: 4000}, (_, i) => `## ${i}\n\n- 变更 ${i}`).join('\n\n'));
  const short = shorten(long);
  assert.ok(Buffer.byteLength(short) <= MAX_DOCUMENT);
  assert.ok(short.startsWith('## 0\n'));
  assert.match(short, /Shortened here/);
  assert.ok(!short.includes('�'));
});

test('a signing key is made once and the index it signs verifies against it', async t => {
  const repo = await packaged(t);
  const keyFile = path.join(repo.root, 'keys/index.pem');
  const made = cli(repo.root, ['keygen', '--out', keyFile]);
  assert.equal(made.status, 0, made.all);
  const publicKey = /base64\): (\S+)/.exec(made.out)[1];
  assert.match(cli(repo.root, ['keygen', '--out', keyFile]).err, /already exists/);
  const bytes = Buffer.from(JSON.stringify(indexOf(repo.results)));
  const signature = signIndex(bytes, fs.readFileSync(keyFile, 'utf8'), 'test-1');
  assert.equal(verifyIndex(bytes, signature, {'test-1': publicKey}).ok, true);
});

test('each index continues the last: the sequence grows and a published version cannot change', async t => {
  const repo = await packaged(t);
  const first = indexOf(repo.results);

  // A new version of one extension is added; the old one stays listed.
  const newer = await takeIn(repo, 'loom.theme.demo', '1.1.0');
  const next = indexOf([newer], {previous: first});
  assert.equal(next.sequence, 2);
  const versions = next.extensions.find(extension => extension.id === 'loom.theme.demo').versions.map(version => version.version);
  assert.deepEqual(versions, ['1.1.0', '1.0.0']);
  assert.deepEqual(validateIndex(next), []);

  // The same version with other contents is refused.
  const changed = await takeIn(repo, 'loom.lang.demo', '1.0.0', folder => write(folder, 'queries/highlights.scm', '(identifier) @function'));
  assert.throws(() => indexOf([changed], {previous: first}), /already published with different contents/);
});

test('an index with the wrong shape is described problem by problem', () => {
  const index = buildIndexForTest();
  index.sequence = 0;
  index.expires = index.generated;
  index.extensions[0].versions[0].artifacts[0].url = 'http://insecure';
  index.extensions[0].versions[0].artifacts.push({platform: 'linux-x86_64', url: 'https://x', sha256: 'a'.repeat(64), size: 1});
  const text = validateIndex(index).join('\n');
  assert.match(text, /sequence must be a positive integer/);
  assert.match(text, /expires must be after generated/);
  assert.match(text, /must be https/);
  assert.match(text, /either for any platform or for named ones/);
});

function buildIndexForTest() {
  return {
    schema: 1, sequence: 1, generated: NOW, expires: '2026-12-01T00:00:00.000Z', keyId: 'k',
    extensions: [{id: 'loom.lang.x', versions: [{version: '1.0.0', engines: {loom: '*'}, artifacts: [{platform: 'any', url: 'https://x', sha256: 'a'.repeat(64), size: 1}]}]}],
  };
}

test('lock pins the bundled extensions to the signed index and keeps a copy of it', async t => {
  const repo = await packaged(t);
  assert.match(cli(repo.root, ['lock']).err, /No index/);
  leaveIndex(repo, indexOf(repo.results));
  assert.match(cli(repo.root, ['lock']).err, /not signed/);
  leaveIndex(repo, indexOf(repo.results), generateKeyPair().privatePem);
  const locked = cli(repo.root, ['lock']);
  assert.equal(locked.status, 0, locked.all);
  const lock = read(repo, 'dist/lock/extensions.lock');
  assert.equal(lock.schema, 1);
  assert.deepEqual(lock.extensions.map(extension => `${extension.id}@${extension.version}`), ['loom.theme.demo@1.0.0']);
  assert.equal(lock.extensions[0].artifacts[0].platform, 'any');
  assert.equal(lock.index.sequence, 1);
  const snapshot = fs.readFileSync(path.join(repo.root, 'dist/lock/index.snapshot.json'));
  assert.equal(lock.index.sha256, sha(snapshot));
  assert.ok(fs.existsSync(path.join(repo.root, 'dist/lock/index.snapshot.json.sig')));
});

test('a bundled extension that is not in the index stops the lock', async t => {
  const repo = await packaged(t, {bundled: ['loom.theme.demo', 'loom.lang.ghost']});
  leaveIndex(repo, indexOf(repo.results), generateKeyPair().privatePem);
  assert.match(cli(repo.root, ['lock']).err, /loom\.lang\.ghost is bundled but not in the index/);
});

test('the validator output is read as one object per grammar, and its failure is reported', () => {
  const exec = output => () => output;
  assert.deepEqual(runValidator('v', 'dir', null, exec('{"grammar":"a","colored":3}\n{"grammar":"b","colored":0}\n')),
    [{grammar: 'a', colored: 3}, {grammar: 'b', colored: 0}]);
  const failing = () => { const error = new Error('x'); error.stderr = 'query error in highlights.scm'; throw error; };
  assert.throws(() => runValidator('v', path.join('x', 'loom.lang.a'), null, failing), /loom\.lang\.a: query error in highlights\.scm/);
});

test('the index builder accepts previous data without packages of the same extension', () => {
  const index = buildIndex({packages: [], repository: 'acme/x', keyId: 'k', validityDays: 1, now: new Date(NOW), previous: buildIndexForTest()});
  assert.equal(index.sequence, 2);
  assert.deepEqual(index.extensions.map(extension => extension.id), ['loom.lang.x']);
});

test('draft creation keeps its returned ID and ID publication does not rediscover a tag', () => {
  const calls = []; const draft = {id:123,tag_name:'loom.lang.demo-0.1.0',draft:true,assets:[]};
  const github = new GitHubReleases('acme/extensions',args=>{
    calls.push(args);
    if(args.includes('POST')) {
      assert.ok(args.includes('tag_name='+draft.tag_name));
      assert.ok(args.includes('draft=true')); assert.ok(args.includes('make_latest=false'));
      assert.ok(args.includes('target_commitish='+'a'.repeat(40)));
      return JSON.stringify(draft);
    }
    assert.deepEqual(args,['api','--method','PATCH','repos/acme/extensions/releases/123','-F','draft=false']);
    return '{}';
  });
  assert.deepEqual(github.createDraft(draft.tag_name,'Demo','Notes',{target:'a'.repeat(40)}),draft);
  github.publish(draft.tag_name,{id:draft.id});
  assert.equal(calls.length,2);
  assert.throws(()=>github.publish(draft.tag_name,{id:'bad'}),/Invalid release ID/);
});
