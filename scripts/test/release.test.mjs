import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import {execFileSync, spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {checkout, populate, cli, write} from './cli.mjs';
import {generateKeyPair, signIndex, verifyIndex} from '../lib/sign.mjs';
import {buildIndex} from '../lib/index.mjs';
import {planRelease, definitions, fingerprint, changedExtensions, readRegistry, validateStaged,
  checkAssets, checkRecords, prepareRelease, finishRelease, readyToIndex} from '../lib/release.mjs';

const commit = 'a'.repeat(40);
const empty = {schema: 1, sequence: 3, keyId: 'test-1', generated: '2026-11-01T00:00:00Z',
  expires: '2026-12-01T00:00:00Z', extensions: []};
const definition = (id = 'loom.lang.demo', version = '1.0.0') => ({id, version, platforms: ['any'], fingerprint: 'b'.repeat(64)});
const indexed = item => ({...empty, extensions: [{id: item.id, versions: [{version: item.version, source: {fingerprint: item.fingerprint}}]}]});

function fixture(t) {
  const repo = checkout(t); populate(repo, ['theme']);
  return {...repo, dist: path.join(repo.root, 'dist'), work: path.join(repo.root, '.work')};
}

test('pending versions include first delivery and upgrades, while published versions cannot be changed or downgraded', () => {
  const item = definition();
  assert.deepEqual(planRelease([item], empty, new Set(), commit).pending, [item]);
  const previous = indexed(item);
  assert.deepEqual(planRelease([item], previous).skipped, [item.id]);
  assert.throws(() => planRelease([{...item, fingerprint: 'c'.repeat(64)}], previous), /bump extension.version/);
  assert.throws(() => planRelease([item], previous, new Set([item.id])), /bump extension.version/);
  assert.equal(planRelease([definition(item.id, '1.0.1')], previous).pending.length, 1);
  assert.throws(() => planRelease([definition(item.id, '0.9.0')], previous), /must be newer/);
});

test('Git comparison includes deleted packaging inputs and excludes Markdown documentation', t => {
  const repo = fixture(t);
  const git = (...args) => execFileSync('git', args, {cwd: repo.root, encoding: 'utf8'}).trim();
  git('init', '-b', 'main'); git('config', 'user.name', 'Release test'); git('config', 'user.email', 'release@example.invalid');
  git('add', '.'); git('commit', '-m', 'fixture'); const base = git('rev-parse', 'HEAD');
  const original = fingerprint(repo.root, 'loom.theme.demo');
  write(repo.extensions, 'loom.theme.demo/README.md', '# Updated documentation');
  assert.equal(fingerprint(repo.root, 'loom.theme.demo'), original);
  git('add', '.'); git('commit', '-m', 'docs'); const docs = git('rev-parse', 'HEAD');
  assert.equal(changedExtensions(repo.root, base, docs).size, 0);
  fs.rmSync(path.join(repo.extensions, 'loom.theme.demo/icon.svg'));
  git('add', '.'); git('commit', '-m', 'remove input'); const head = git('rev-parse', 'HEAD');
  assert.deepEqual([...changedExtensions(repo.root, docs, head)], ['loom.theme.demo']);
  assert.equal(fingerprint(repo.root, 'loom.theme.demo', base), original);
  assert.notEqual(fingerprint(repo.root, 'loom.theme.demo'), original);
});

test('registry failures and invalid signatures never become an empty index', async () => {
  const keys = generateKeyPair();
  const config = {repository: 'acme/extensions', index: {keys: {'test-1': keys.publicRaw}}};
  const bytes = Buffer.from(JSON.stringify(empty));
  const signature = signIndex(bytes, keys.privatePem, 'test-1');
  const response = value => new Response(value, {status: 200});
  const fetcher = async url => response(url.endsWith('.sig') ? signature : bytes);
  assert.equal((await readRegistry(config, fetcher)).index.sequence, 3);
  await assert.rejects(readRegistry(config, async () => new Response('', {status: 503})), /HTTP 503/);
  await assert.rejects(readRegistry(config, async url => response(url.endsWith('.sig') ? signature : JSON.stringify({...empty, sequence: 4}))), /signature/);
  await assert.rejects(readRegistry({...config, index: {keys: {}}}, fetcher), /unknown key/);
});

test('handwritten languages require the real validator and every declared platform must be staged', t => {
  const repo = checkout(t); populate(repo, ['language']);
  assert.equal(cli(repo.root, ['build']).status, 0);
  const item = definitions(repo.root)[0]; let calls = 0;
  const options = {root: repo.root, dist: path.join(repo.root, 'dist'), validator: 'test-validator', validate: () => {calls++; return [];}};
  validateStaged(item, options); assert.equal(calls, 1);
  assert.throws(() => validateStaged(item, {...options, validator: null}), /require loom-ext-validate/);
  assert.throws(() => validateStaged({...item, platforms: ['any', 'linux-x86_64']}, options), /differ from the recipe/);
  assert.throws(() => validateStaged(item, {...options, validate: () => {throw Error('bad query');}}), /bad query/);
});

test('preparation checks packaging inputs and local package bytes before publication', async t => {
  const repo = fixture(t), item = definitions(repo.root)[0];
  const prepared = await prepareRelease(item, {...repo, validator: 'test-validator', repository: 'acme/extensions', lookup: async () => null});
  checkRecords(item, prepared.records, path.join(repo.dist, 'packages'));
  assert.equal(prepared.description.manifest.id, item.id);
  fs.appendFileSync(path.join(repo.dist, 'packages', prepared.records[0].file), 'corrupt');
  assert.throws(() => checkRecords(item, prepared.records, path.join(repo.dist, 'packages')), /does not match/);
  write(repo.extensions, 'loom.theme.demo/icon.svg', 'changed');
  await assert.rejects(prepareRelease(item, {...repo}), /inputs changed/);
});

test('a public release missing from the index is restored from original bytes, not rebuilt', async t => {
  const repo = fixture(t), item = definitions(repo.root)[0];
  const first = await prepareRelease(item, {...repo, validator: 'test-validator', repository: 'acme/extensions', lookup: async () => null});
  const original = fs.readFileSync(path.join(repo.dist, 'packages', first.records[0].file));
  const remote = {draft: false, assets: first.records.map(r => ({name: r.file, size: r.size, digest: `sha256:${r.sha256}`, browser_download_url: 'https://example.invalid/package'}))};
  const recovered = await prepareRelease(item, {...repo, validator: 'test-validator', repository: 'acme/extensions', lookup: async () => remote,
    download: async (url, sha, target) => fs.writeFileSync(target, original)});
  assert.equal(recovered.recovered, true); assert.deepEqual(recovered.records, first.records);
  assert.deepEqual(fs.readFileSync(path.join(repo.dist, 'packages', first.records[0].file)), original);
  checkAssets(remote, first.records);
  assert.throws(() => checkAssets({...remote, assets: [{...remote.assets[0], digest: undefined}]}, first.records), /SHA-256/);
});

function publication(t) {
  const repo = fixture(t), keys = generateKeyPair();
  const config = {repository: 'acme/extensions', bundled: ['loom.theme.demo'], index: {'key-id': 'test-1', 'validity-days': 30, keys: {'test-1': keys.publicRaw}}};
  const remote = new Map(); const calls = [];
  let registry = {index: empty, bytes: Buffer.from(JSON.stringify(empty)), signature: signIndex(Buffer.from(JSON.stringify(empty)), keys.privatePem, 'test-1')};
  const releases = {
    createDraft(tag, title, notes, options) {assert.equal(options.target, commit); calls.push('create'); remote.set(tag, {draft: true, assets: []});},
    upload(tag, files) {
      calls.push(tag === 'registry' ? (path.basename(files[0]) === 'index.json' ? 'index' : 'generation') : 'packages');
      if (tag === 'registry') {
        const bytes = fs.readFileSync(files[0]), signature = fs.readFileSync(files[1], 'utf8');
        assert.equal(verifyIndex(bytes, signature, config.index.keys).ok, true);
        registry = {bytes, signature, index: JSON.parse(bytes), maxSequence: JSON.parse(bytes).sequence, recovered: path.basename(files[0]) !== 'index.json'};
      } else remote.get(tag).assets = files.map(file => {
        const bytes = fs.readFileSync(file);
        return {name: path.basename(file), size: bytes.length, digest: `sha256:${hashForTest(bytes)}`};
      });
    },
    publish(tag) {calls.push('publish'); remote.get(tag).draft = false;},
  };
  return {...repo, keys, config, remote, calls, releases, lookup: async (repository, tag) => remote.get(tag) ?? null, registry: async () => registry};
}
import {createHash} from 'node:crypto';
import {clearStage} from '../lib/stage.mjs';
const hashForTest = bytes => createHash('sha256').update(bytes).digest('hex');

test('publication verifies complete assets before visibility, preserves previous entries and recovers after interruption', async t => {
  const ctx = publication(t), item = definitions(ctx.root)[0];
  const prepared = await prepareRelease(item, {...ctx, validator: 'test-validator'});
  clearStage(item.id, ctx.dist); // CI frees staging before the publisher starts.
  const plan = planRelease([item], empty, new Set(), commit);
  const options = {...ctx, privateKey: ctx.keys.privatePem, sourceRevision: commit};
  await assert.rejects(finishRelease(plan, [prepared], {...options, privateKey: null}), /LOOM_INDEX_KEY/);
  assert.deepEqual(ctx.calls, []);
  await assert.rejects(finishRelease(plan, [prepared], {...options, privateKey: generateKeyPair().privatePem}), /trusted public key/);
  assert.deepEqual(ctx.calls, []);
  const goodRegistry = ctx.registry;
  let reads = 0;
  const broken = async config => {if (++reads > 3) throw Error('readback interrupted'); return goodRegistry(config);};
  await assert.rejects(finishRelease(plan, [prepared], {...options, registry: broken}), /interrupted/);
  assert.equal(ctx.remote.get('loom.theme.demo-1.0.0').draft, false);
  assert.deepEqual(ctx.calls, ['create', 'packages', 'publish', 'generation', 'index']);
  ctx.calls.length = 0;
  const report = await finishRelease(plan, [prepared], options);
  assert.equal(report.failures.length, 0);
  assert.deepEqual(ctx.calls, [], 'a completed release and index are never rewritten');
  assert.equal((await goodRegistry()).index.extensions[0].versions[0].source.revision, commit);
});

test('failed builds do not hide successful independent extensions, and missing results are failures', async t => {
  const ctx = publication(t), item = definitions(ctx.root)[0];
  const prepared = await prepareRelease(item, {...ctx, validator: 'test-validator'});
  const missing = definition('loom.lang.missing'), failed = definition('loom.lang.failed');
  const plan = planRelease([item, missing, failed], empty, new Set(), commit);
  const report = await finishRelease(plan, [prepared, {id: failed.id, error: 'build failed'}], {...ctx, privateKey: ctx.keys.privatePem});
  assert.deepEqual(report.indexed, [item.id]);
  assert.deepEqual(report.failures.map(e => e.id).sort(), [failed.id, missing.id]);
});

test('a dependent version is withheld until a compatible runtime is available', () => {
  const runtime = {id: 'loom.runtime.node', version: '22.0.0', description: {manifest: {}}};
  const server = {id: 'loom.lsp.demo', version: '1.0.0', description: {manifest: {requires: ['loom.runtime.node@^22']}}};
  assert.deepEqual(readyToIndex([server], empty), []);
  const cyclic = {...runtime, description: {manifest: {requires: ['loom.lsp.demo@^1']}}};
  assert.deepEqual(readyToIndex([server, cyclic], empty), [], 'dependency cycles cannot be published');
  assert.deepEqual(readyToIndex([runtime, server], empty), [runtime, server]);
  assert.deepEqual(readyToIndex([server], {extensions: [{id: runtime.id, versions: [{version: '22.1.0'}]}]}), [server]);
});

test('automatic workflow gates publication on main, real validation and the signing secret', () => {
  const text = fs.readFileSync(new URL('../../.github/workflows/release.yml', import.meta.url), 'utf8');
  assert.match(text, /push:\s+branches: \[main\]/);
  assert.match(text, /github.ref == 'refs\/heads\/main'/);
  assert.match(text, /cancel-in-progress: false/);
  assert.match(text, /release plan/); assert.match(text, /release prepare/); assert.match(text, /release publish/);
  assert.doesNotMatch(text, /require-validator:|inputs.extensions|--unsigned/);
  assert.match(text, /LOOM_INDEX_KEY: \$\{\{ secrets.LOOM_INDEX_KEY \}\}/);
  assert.match(text, /if: always\(\)/);
});

test('a mismatched remote upload stays a draft and never enters the index', async t => {
  const ctx = publication(t), item = definitions(ctx.root)[0];
  const prepared = await prepareRelease(item, {...ctx, validator: 'test-validator'});
  const upload = ctx.releases.upload;
  ctx.releases.upload = (tag, files, options) => {
    upload(tag, files, options);
    if (tag !== 'registry') ctx.remote.get(tag).assets[0].digest = 'sha256:' + '0'.repeat(64);
  };
  const report = await finishRelease(planRelease([item], empty, new Set(), commit), [prepared], {...ctx, privateKey: ctx.keys.privatePem});
  assert.equal(report.failures.length, 1);
  assert.equal(ctx.remote.get('loom.theme.demo-1.0.0').draft, true);
  assert.ok(!ctx.calls.includes('publish') && !ctx.calls.includes('index'));
});

test('the validator bootstrap runs against the configured root and emits a pinned source or binary', t => {
  const repo = checkout(t);
  write(repo.root, 'tools/validator-source.toml', 'schema = 1\nrepository = "acme/loom"\nrevision = "' + commit + '"\n');
  const output = path.join(repo.root, 'outputs');
  const run = extra => spawnSync(process.execPath, [fileURLToPath(new URL('../validator-source.mjs', import.meta.url))], {
    cwd: repo.root, encoding: 'utf8', env: {...process.env, LOOM_EXT_ROOT: repo.root, LOOM_EXT_VALIDATE: '', GITHUB_OUTPUT: output, ...extra}
  });
  let result = run();
  assert.equal(result.status, 0, result.stderr);
  assert.equal(fs.readFileSync(output, 'utf8'), 'source=true\nrepository=acme/loom\nrevision=' + commit + '\n');
  fs.writeFileSync(output, '');
  const binary = write(repo.root, 'provided-validator', 'test binary');
  result = run({LOOM_EXT_VALIDATE: binary});
  assert.equal(result.status, 0, result.stderr);
  assert.equal(fs.readFileSync(output, 'utf8'), 'source=false\n');
  assert.equal(fs.readFileSync(path.join(repo.root, 'tools/bin', process.platform === 'win32' ? 'loom-ext-validate.exe' : 'loom-ext-validate'), 'utf8'), 'test binary');
});

test('a mismatched current index/signature pair recovers the newest verified immutable generation', async () => {
  const keys = generateKeyPair();
  const config = {repository: 'acme/extensions', index: {keys: {'test-1': keys.publicRaw}}};
  const bytes = sequence => JSON.stringify({...empty, sequence});
  const signature = sequence => signIndex(Buffer.from(bytes(sequence)), keys.privatePem, 'test-1');
  const assets = ['index-4.json', 'index-4.json.sig', 'index-5.json.sig'].map(name => ({name}));
  const responses = {'index.json': bytes(4), 'index.json.sig': signature(3), 'index-4.json': bytes(4), 'index-4.json.sig': signature(4)};
  const fetcher = async url => url.startsWith('https://api.github.com/')
    ? new Response(JSON.stringify({assets})) : new Response(responses[url.split('/').at(-1)] ?? '', {status: responses[url.split('/').at(-1)] ? 200 : 404});
  const recovered = await readRegistry(config, fetcher);
  assert.equal(recovered.index.sequence, 4);
  assert.equal(recovered.maxSequence, 5, 'an incomplete attempted generation is never overwritten');
  assert.equal(recovered.recovered, true);
  responses['index.json.sig'] = signature(4);
  assert.equal((await readRegistry(config, fetcher)).recovered, false);
  responses['index.json'] = JSON.stringify({...empty, sequence: 4, generated: '2026-11-01T00:00:01Z'});
  responses['index.json.sig'] = signIndex(Buffer.from(responses['index.json']), keys.privatePem, 'test-1');
  await assert.rejects(readRegistry(config, fetcher), /Conflicting signed/);
});

test('an interruption before switching the current pair repairs from its generation without a new release or sequence', async t => {
  const ctx = publication(t), item = definitions(ctx.root)[0];
  const prepared = await prepareRelease(item, {...ctx, validator: 'test-validator'});
  const plan = planRelease([item], empty, new Set(), commit);
  const options = {...ctx, privateKey: ctx.keys.privatePem};
  const upload = ctx.releases.upload;
  ctx.releases.upload = (tag, files, options) => {
    if (tag === 'registry' && path.basename(files[0]) === 'index.json') throw Error('current pair interrupted');
    return upload(tag, files, options);
  };
  await assert.rejects(finishRelease(plan, [prepared], options), /pair interrupted/);
  assert.equal((await ctx.registry()).recovered, true);
  const sequence = (await ctx.registry()).index.sequence;
  ctx.calls.length = 0; ctx.releases.upload = upload;
  const report = await finishRelease(plan, [prepared], options);
  assert.equal(report.sequence, sequence);
  assert.deepEqual(ctx.calls, ['index']);
  assert.equal((await ctx.registry()).recovered, false);
});

test('release cleanup removes unfinished platform stages and preserves other extensions', t => {
  const repo = checkout(t), dist = path.join(repo.root, 'dist');
  write(dist, 'stage/any/loom.lang.demo/partial.wasm', 'partial build');
  write(dist, 'stage/linux-x86_64/loom.lang.demo/manifest.json', '{}');
  const other = write(dist, 'stage/any/loom.lang.other/resource', 'keep');
  clearStage('loom.lang.demo', dist);
  assert.equal(fs.existsSync(path.join(dist, 'stage/any/loom.lang.demo')), false);
  assert.equal(fs.existsSync(path.join(dist, 'stage/linux-x86_64/loom.lang.demo')), false);
  assert.equal(fs.readFileSync(other, 'utf8'), 'keep');
});

test('new indexes advance beyond the reserved sequence of an incomplete generation', async t => {
  const ctx = publication(t), item = definitions(ctx.root)[0];
  const prepared = await prepareRelease(item, {...ctx, validator: 'test-validator'});
  (await ctx.registry()).maxSequence = 7;
  const report = await finishRelease(planRelease([item], empty, new Set(), commit), [prepared], {...ctx, privateKey: ctx.keys.privatePem});
  assert.equal(report.sequence, 8);
});
