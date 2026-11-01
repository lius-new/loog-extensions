import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import {createHash} from 'node:crypto';
import {generateKeyPair, publicKeyOf, signIndex, verifyIndex} from '../lib/sign.mjs';
import {buildIndex, validateIndex} from '../lib/index.mjs';
import {GitHubReleases} from '../lib/github.mjs';
import {run as publish, planReleases} from '../commands/publish.mjs';
import {runValidator} from '../lib/validator.mjs';
import {checkout, cli, populate, write} from './cli.mjs';

const NOW = '2026-11-01T00:00:00.000Z';
const sha = bytes => createHash('sha256').update(bytes).digest('hex');
const read = (repo, file) => JSON.parse(fs.readFileSync(path.join(repo.root, file), 'utf8'));

function built(t, what = ['theme', 'language']) {
  const repo = checkout(t, {bundled: ['loom.theme.demo']});
  populate(repo, what);
  assert.equal(cli(repo.root, ['build']).status, 0);
  assert.equal(cli(repo.root, ['pack']).status, 0);
  return repo;
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

test('the index lists each extension with its packages and what Loom needs to choose', t => {
  const repo = built(t);
  const indexed = cli(repo.root, ['index', '--unsigned', '--now', NOW]);
  assert.equal(indexed.status, 0, indexed.all);
  const index = read(repo, 'dist/index/index.json');
  assert.deepEqual(validateIndex(index), []);
  assert.equal(index.sequence, 1);
  assert.equal(index.generated, NOW);
  assert.equal(index.expires, '2026-12-01T00:00:00.000Z');
  assert.equal(index.keyId, 'test-1');
  assert.deepEqual(index.extensions.map(extension => extension.id), ['loom.lang.demo', 'loom.theme.demo']);
  const language = index.extensions[0];
  assert.deepEqual(language.languages, [{name: 'demo', fileTypes: ['demo']}]);
  const version = language.versions[0];
  assert.equal(version.version, '1.0.0');
  assert.deepEqual(version.engines, {loom: '>=0.1.8 <0.3'});
  assert.deepEqual(version.artifacts.map(artifact => artifact.platform), ['any']);
  assert.equal(version.artifacts[0].url, 'https://github.com/acme/extensions/releases/download/loom.lang.demo-1.0.0/loom.lang.demo-1.0.0-any.loomext');
  assert.ok(!fs.existsSync(path.join(repo.root, 'dist/index/index.json.sig')), 'unsigned');
});

test('an index is signed with the key given and refused without one', t => {
  const repo = built(t);
  assert.match(cli(repo.root, ['index']).err, /Sign the index with --key/);
  const keyFile = path.join(repo.root, 'keys/index.pem');
  const made = cli(repo.root, ['keygen', '--out', keyFile]);
  assert.equal(made.status, 0, made.all);
  const publicKey = /base64\): (\S+)/.exec(made.out)[1];
  assert.match(cli(repo.root, ['keygen', '--out', keyFile]).err, /already exists/);
  assert.equal(cli(repo.root, ['index', '--key', keyFile]).status, 0);
  const bytes = fs.readFileSync(path.join(repo.root, 'dist/index/index.json'));
  const signature = fs.readFileSync(path.join(repo.root, 'dist/index/index.json.sig'), 'utf8');
  assert.equal(verifyIndex(bytes, signature, {'test-1': publicKey}).ok, true);
  // The environment variable carries a key as well.
  assert.equal(cli(repo.root, ['index'], {LOOM_INDEX_KEY: fs.readFileSync(keyFile, 'utf8')}).status, 0);
});

test('each index continues the last: the sequence grows and a published version cannot change', t => {
  const repo = built(t);
  cli(repo.root, ['index', '--unsigned', '--now', NOW]);
  const first = path.join(repo.root, 'first-index.json');
  fs.copyFileSync(path.join(repo.root, 'dist/index/index.json'), first);

  // A new version of one extension is added; the old one stays listed.
  const theme = path.join(repo.extensions, 'loom.theme.demo/manifest.json');
  write(path.dirname(theme), 'manifest.json', {...JSON.parse(fs.readFileSync(theme, 'utf8')), version: '1.1.0'});
  assert.equal(cli(repo.root, ['build', 'loom.theme.demo']).status, 0);
  assert.equal(cli(repo.root, ['pack', 'loom.theme.demo']).status, 0);
  assert.equal(cli(repo.root, ['index', '--unsigned', '--previous', first]).status, 0);
  const next = read(repo, 'dist/index/index.json');
  assert.equal(next.sequence, 2);
  const versions = next.extensions.find(extension => extension.id === 'loom.theme.demo').versions.map(version => version.version);
  assert.deepEqual(versions, ['1.1.0', '1.0.0']);
  assert.deepEqual(validateIndex(next), []);

  // The same version with other contents is refused.
  write(repo.extensions, 'loom.lang.demo/queries/highlights.scm', '(identifier) @function');
  cli(repo.root, ['build', 'loom.lang.demo']);
  cli(repo.root, ['pack', 'loom.lang.demo']);
  assert.match(cli(repo.root, ['index', '--unsigned', '--previous', first]).err, /already published with different contents/);
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

test('lock pins the bundled extensions to the signed index and keeps a copy of it', t => {
  const repo = built(t);
  assert.match(cli(repo.root, ['lock']).err, /No index/);
  cli(repo.root, ['index', '--unsigned', '--now', NOW]);
  assert.match(cli(repo.root, ['lock']).err, /not signed/);
  const keyFile = path.join(repo.root, 'index.pem');
  cli(repo.root, ['keygen', '--out', keyFile]);
  cli(repo.root, ['index', '--key', keyFile, '--now', NOW]);
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

test('a bundled extension that is not in the index stops the lock', t => {
  const repo = checkout(t, {bundled: ['loom.theme.demo', 'loom.lang.ghost']});
  populate(repo);
  cli(repo.root, ['build', 'loom.theme.demo']);
  cli(repo.root, ['pack', 'loom.theme.demo']);
  const keyFile = path.join(repo.root, 'index.pem');
  cli(repo.root, ['keygen', '--out', keyFile]);
  cli(repo.root, ['index', '--key', keyFile]);
  assert.match(cli(repo.root, ['lock']).err, /loom\.lang\.ghost is bundled but not in the index/);
});

// ---------------------------------------------------------------- publishing

/** A `gh` stand-in that keeps releases in memory. */
function fakeGh(initial = {}) {
  const releases = structuredClone(initial);
  const calls = [];
  const run = args => {
    calls.push(args.slice(0, 2).join(' ') + ' ' + (args[2] ?? ''));
    const [, command, tag] = [args[0], args[1], args[2]];
    if (args[0] === 'api' && args.includes('POST')) {
      const tag = args.find(arg => arg.startsWith('tag_name=')).slice('tag_name='.length);
      releases[tag] = {id: Object.keys(releases).length + 1, tag_name: tag, draft: true, assets: []};
      return JSON.stringify(releases[tag]);
    }
    if (args[0] === 'release' && command === 'view') {
      if (!releases[tag]) { const error = new Error('release not found'); error.stderr = 'release not found'; throw error; }
      return JSON.stringify({tagName: tag, isDraft: releases[tag].draft, assets: releases[tag].assets});
    }
    if (command === 'create') { releases[tag] = {draft: args.includes('--draft'), assets: []}; return ''; }
    if (command === 'upload') {
      const files = args.slice(3).filter(arg => !arg.startsWith('--') && !arg.includes('/') || /\.(loomext|json|sig)$/.test(arg));
      for (const file of files) {
        const bytes = fs.readFileSync(file);
        const asset = {name: path.basename(file), size: bytes.length, digest: `sha256:${sha(bytes)}`};
        releases[tag].assets = releases[tag].assets.filter(existing => existing.name !== asset.name).concat(asset);
      }
      return '';
    }
    if (command === 'edit') { releases[tag].draft = false; return ''; }
    throw new Error(`unexpected gh ${args.join(' ')}`);
  };
  return {run, releases, calls};
}

test('publishing is a dry run until --yes, then creates, uploads, checks and publishes', async t => {
  const repo = built(t);
  cli(repo.root, ['index', '--unsigned']);
  const gh = fakeGh();
  const github = new GitHubReleases('acme/extensions', gh.run);
  const where = {releases: github, dist: path.join(repo.root, 'dist')};
  await publish([], where);
  assert.deepEqual(Object.keys(gh.releases), [], 'a dry run changes nothing');
  await publish(['--yes'], where);
  assert.deepEqual(Object.keys(gh.releases).sort(), ['loom.lang.demo-1.0.0', 'loom.theme.demo-1.0.0', 'registry']);
  assert.equal(gh.releases['loom.theme.demo-1.0.0'].draft, false);
  assert.deepEqual(gh.releases['loom.theme.demo-1.0.0'].assets.map(asset => asset.name), ['loom.theme.demo-1.0.0-any.loomext']);
  assert.deepEqual(gh.releases.registry.assets.map(asset => asset.name), ['index.json']);
});

test('a published release with other contents stops publishing, and a matching one is skipped', t => {
  const records = [{id: 'loom.theme.demo', version: '1.0.0', platform: 'any', file: 'loom.theme.demo-1.0.0-any.loomext', sha256: 'a'.repeat(64), size: 10}];
  const published = (digest, size = 10) => new GitHubReleases('acme/x', fakeGh({'loom.theme.demo-1.0.0': {draft: false, assets: [{name: records[0].file, size, digest: `sha256:${digest}`}]}}).run);
  assert.equal(planReleases(records, published('a'.repeat(64)))[0].action, 'skip');
  assert.throws(() => planReleases(records, published('b'.repeat(64))), /already published with different contents/);
  assert.throws(() => planReleases(records, published('a'.repeat(64), 11)), /size 11, expected 10/);
  const draft = new GitHubReleases('acme/x', fakeGh({'loom.theme.demo-1.0.0': {draft: true, assets: []}}).run);
  assert.deepEqual(planReleases(records, draft)[0], {...planReleases(records, draft)[0], action: 'resume', upload: [records[0].file]});
  const fresh = new GitHubReleases('acme/x', fakeGh().run);
  assert.equal(planReleases(records, fresh)[0].action, 'create');
});

// ---------------------------------------------------------------- validating

test('validate checks staged packages, and needs build first', t => {
  const repo = checkout(t, {bundled: []});
  populate(repo);
  assert.match(cli(repo.root, ['validate', '--offline']).err, /nothing is staged/);
  cli(repo.root, ['build']);
  const ok = cli(repo.root, ['validate', '--offline']);
  assert.equal(ok.status, 0, ok.all);
  assert.match(ok.err, /loom-ext-validate was not found/);
  assert.match(cli(repo.root, ['validate', '--offline', '--require-validator']).err, /is required but was not found/);
  fs.rmSync(path.join(repo.root, 'dist/stage/any/loom.theme.demo/themes/dark.json'));
  assert.match(cli(repo.root, ['validate', '--offline']).err, /themes\/dark\.json does not exist/);
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
