import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import {execFileSync, spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {checkout, populate, write} from './cli.mjs';
import {generateKeyPair, signIndex, verifyIndex} from '../lib/sign.mjs';
import {createPackage} from '../lib/archive.mjs';
import {intake} from '../lib/intake.mjs';
import {fingerprint, readRegistry, readRelease, checkAssets, checkRecords, finishRelease, readyToIndex, verifyRelease} from '../lib/release.mjs';

const commit = 'a'.repeat(40);
const empty = {schema: 1, sequence: 3, keyId: 'test-1', generated: '2026-11-01T00:00:00Z',
  expires: '2026-12-01T00:00:00Z', extensions: []};
const definition = (id = 'loom.lang.demo', version = '1.0.0') => ({id, version, platforms: ['any'], fingerprint: 'b'.repeat(64)});

function fixture(t) {
  const repo = checkout(t); populate(repo, ['theme']);
  // Inputs are what git sees, so the fixture is a repository.
  execFileSync('git', ['init', '-q'], {cwd: repo.root});
  return {...repo, dist: path.join(repo.root, 'dist'), work: path.join(repo.root, '.work')};
}

/** The demo theme packaged "elsewhere", taken in, and the plan that goes with it. */
async function delivery(repo, id = 'loom.theme.demo') {
  const folder = path.join(repo.root, 'out', id);
  fs.cpSync(path.join(repo.extensions, id), folder, {recursive: true});
  const file = path.join(repo.root, 'handed-over', `${id}-1.0.0-any.loomext`);
  await createPackage(folder, file, id);
  const [prepared] = await intake([file], {...repo, validator: 'test-validator', validate: () => []});
  assert.equal(prepared.error, undefined);
  const item = {id, version: prepared.version, fingerprint: prepared.source.fingerprint, platforms: ['any']};
  return {item, prepared, plan: {schema: 1, revision: commit, sequence: 3, pending: [item], skipped: []}};
}

test('a fingerprint follows the files git sees, leaves out Markdown, and notices a deleted input', t => {
  const repo = fixture(t);
  const git = (...args) => execFileSync('git', args, {cwd: repo.root, encoding: 'utf8'}).trim();
  git('config', 'user.name', 'Release test'); git('config', 'user.email', 'release@example.invalid');
  git('add', '.'); git('commit', '-m', 'fixture'); const base = git('rev-parse', 'HEAD');
  const original = fingerprint(repo.root, 'loom.theme.demo');
  write(repo.extensions, 'loom.theme.demo/README.md', '# Updated documentation');
  assert.equal(fingerprint(repo.root, 'loom.theme.demo'), original);
  write(repo.root, '.gitignore', 'node_modules/\n');
  write(repo.extensions, 'loom.theme.demo/node_modules/dependency/index.js', 'ignored');
  assert.equal(fingerprint(repo.root, 'loom.theme.demo'), original);
  fs.rmSync(path.join(repo.extensions, 'loom.theme.demo/icon.svg'));
  assert.equal(fingerprint(repo.root, 'loom.theme.demo', base), original);
  assert.notEqual(fingerprint(repo.root, 'loom.theme.demo'), original);
});

test('registry failures and invalid signatures never become an empty index', async () => {
  const keys = generateKeyPair();
  const config = {repository: 'acme/extensions', index: {keys: {'test-1': keys.publicRaw}}};
  const bytes = Buffer.from(JSON.stringify(empty));
  const signature = signIndex(bytes, keys.privatePem, 'test-1');
  const response = value => new Response(value, {status: 200});
  const fetcher = async url => response(new URL(url).pathname.endsWith('.sig') ? signature : bytes);
  assert.equal((await readRegistry(config, fetcher)).index.sequence, 3);
  await assert.rejects(readRegistry(config, async () => new Response('', {status: 503})), /HTTP 503/);
  await assert.rejects(readRegistry(config, async url => response(new URL(url).pathname.endsWith('.sig') ? signature : JSON.stringify({...empty, sequence: 4}))), /signature/);
  await assert.rejects(readRegistry({...config, index: {keys: {}}}, fetcher), /unknown key/);
});

test('the package bytes are checked against their records before and after publication', async t => {
  const repo = fixture(t), {item, prepared} = await delivery(repo);
  checkRecords(item, prepared.records, path.join(repo.dist, 'packages'));
  assert.equal(prepared.description.manifest.id, item.id);
  const remote = {draft: false, assets: prepared.records.map(r => ({name: r.file, size: r.size, digest: `sha256:${r.sha256}`}))};
  checkAssets(remote, prepared.records);
  assert.throws(() => checkAssets({...remote, assets: [{...remote.assets[0], digest: undefined}]}, prepared.records), /SHA-256/);
  fs.appendFileSync(path.join(repo.dist, 'packages', prepared.records[0].file), 'corrupt');
  assert.throws(() => checkRecords(item, prepared.records, path.join(repo.dist, 'packages')), /does not match/);
});

function publication(t) {
  const repo = fixture(t), keys = generateKeyPair();
  const config = {repository: 'acme/extensions', bundled: ['loom.theme.demo'], index: {'key-id': 'test-1', 'validity-days': 30, keys: {'test-1': keys.publicRaw}}};
  const remote = new Map(); const calls = [];
  let registry = {index: empty, bytes: Buffer.from(JSON.stringify(empty)), signature: signIndex(Buffer.from(JSON.stringify(empty)), keys.privatePem, 'test-1')};
  const releases = {
    listTags: () => [],
    createDraft(tag, title, notes, options) {assert.equal(options.target, commit); calls.push('create'); const release = {id: remote.size + 1, tag_name: tag, draft: true, assets: []}; remote.set(tag, release); return release;},
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
  return {...repo, keys, config, remote, calls, releases, wait: async () => {}, lookup: async (repository, tag) => remote.get(tag) ?? null, registry: async () => registry};
}
import {createHash} from 'node:crypto';
import {clearStage} from '../lib/stage.mjs';
const hashForTest = bytes => createHash('sha256').update(bytes).digest('hex');

test('publication verifies complete assets before visibility, preserves previous entries and recovers after interruption', async t => {
  const ctx = publication(t), {item, prepared, plan} = await delivery(ctx);
  clearStage(item.id, ctx.dist); // CI frees staging before the publisher starts.
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

test('rejected packages do not hide successful independent extensions, and missing results are failures', async t => {
  const ctx = publication(t), {item, prepared, plan: single} = await delivery(ctx);
  const missing = definition('loom.lang.missing'), failed = definition('loom.lang.failed');
  const plan = {...single, pending: [item, missing, failed]};
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

test('publication is one writer at a time, needs the signing secret, and runs intake before publish', () => {
  const text = fs.readFileSync(new URL('../../.github/workflows/publish.yml', import.meta.url), 'utf8');
  assert.match(text, /workflow_call:/);
  assert.match(text, /group: extension-publication\s+cancel-in-progress: false/);
  assert.ok(text.indexOf('release preflight') < text.indexOf('release intake') && text.indexOf('release intake') < text.indexOf('release publish'));
  // GitHub cancels all but the newest waiting run of a concurrency group, so a run publishes
  // every finished build of its commit, not only the one that called it.
  assert.match(text, /actions: read/);
  assert.match(text, /workflow_run\.head_sha == "\$\{\{ github\.sha \}\}"/);
  assert.match(text, /release intake \.work\/handed-over\/\*/);
  assert.doesNotMatch(text, /--unsigned|--dry-run/);
  assert.match(text, /LOOM_INDEX_KEY: \$\{\{ secrets.LOOM_INDEX_KEY \}\}/);
  assert.match(text, /if: always\(\)/);
});

test('every extension builds in its own workflow and hands over to the shared publisher only on main', () => {
  const workflows = new URL('../../.github/workflows/', import.meta.url), extensions = new URL('../../extensions/', import.meta.url);
  for (const id of fs.readdirSync(extensions)) {
    const text = fs.readFileSync(new URL(`ext-${id}.yml`, workflows), 'utf8');
    assert.match(text, new RegExp(`extensions/${id.replaceAll('.', '\\.')}/\\*\\*`), id);
    assert.match(text, /uses: \.\/\.github\/workflows\/publish\.yml/, id);
    assert.match(text, /if: github\.ref == 'refs\/heads\/main'/, id);
    assert.match(text, /secrets: inherit/, id);
    assert.doesNotMatch(text, /LOOM_INDEX_KEY/, 'only the shared publisher sees the signing key');
  }
});

test('a mismatched remote upload stays a draft and never enters the index', async t => {
  const ctx = publication(t), {prepared, plan} = await delivery(ctx);
  const upload = ctx.releases.upload;
  ctx.releases.upload = (tag, files, options) => {
    upload(tag, files, options);
    if (tag !== 'registry') ctx.remote.get(tag).assets[0].digest = 'sha256:' + '0'.repeat(64);
  };
  const report = await finishRelease(plan, [prepared], {...ctx, privateKey: ctx.keys.privatePem});
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
    ? new Response(JSON.stringify({assets})) : new Response(responses[new URL(url).pathname.split('/').at(-1)] ?? '', {status: responses[new URL(url).pathname.split('/').at(-1)] ? 200 : 404});
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
  const ctx = publication(t), {prepared, plan} = await delivery(ctx);
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
  const ctx = publication(t), {prepared, plan} = await delivery(ctx);
  (await ctx.registry()).maxSequence = 7;
  const report = await finishRelease(plan, [prepared], {...ctx, privateKey: ctx.keys.privatePem});
  assert.equal(report.sequence, 8);
});

test('no workflow input or script can delete published extension releases', () => {
  const workflows = new URL('../../.github/workflows/', import.meta.url);
  for (const name of fs.readdirSync(workflows)) assert.doesNotMatch(fs.readFileSync(new URL(name, workflows), 'utf8'), /reset|release delete|--cleanup-tag/i, name);
});


test('release lookup finds drafts through authenticated listings when the published-tag endpoint returns 404', async () => {
  const calls = [];
  const draft = {id: 7, tag_name: 'loom.lang.demo-0.1.0', draft: true, assets: [{name: 'package.loomext'}]};
  const fetcher = async (url, options) => {
    calls.push(url); assert.equal(options.headers['Cache-Control'], 'no-cache');
    const parsed = new URL(url); assert.ok(parsed.searchParams.get('_check'));
    if (parsed.pathname.includes('/tags/')) return new Response('', {status: 404});
    const page = parsed.searchParams.get('page');
    return new Response(JSON.stringify(page === '1' ? Array.from({length: 100}, (_, i) => ({tag_name: 'unrelated-' + i})) : [draft]));
  };
  assert.deepEqual(await readRelease('acme/extensions', draft.tag_name, fetcher), draft);
  assert.equal(calls.length, 3);
  assert.deepEqual(await readRelease('acme/extensions', 'missing', async url => new Response(new URL(url).pathname.includes('/tags/') ? '' : '[]',
    {status: new URL(url).pathname.includes('/tags/') ? 404 : 200})), null);
});

test('draft lookup propagates inventory failures and uses fresh requests for asset readback', async () => {
  let reads = 0; const urls = [];
  const fetcher = async url => {
    urls.push(url);
    if (new URL(url).pathname.includes('/tags/')) return new Response('', {status: 404});
    return new Response(JSON.stringify([{tag_name: 'demo', draft: true, assets: Array(++reads).fill({name: 'asset'})}]));
  };
  assert.equal((await readRelease('acme/extensions', 'demo', fetcher)).assets.length, 1);
  assert.equal((await readRelease('acme/extensions', 'demo', fetcher)).assets.length, 2);
  assert.notEqual(urls[0], urls[2]);
  await assert.rejects(readRelease('acme/extensions', 'demo', async url => new Response('', {status: new URL(url).pathname.includes('/tags/') ? 404 : 503})), /HTTP 503/);
});


test('registry pair reads bypass old CDN aliases without weakening signature verification', async () => {
  const keys = generateKeyPair();
  const config = {repository: 'acme/extensions', index: {keys: {'test-1': keys.publicRaw}}};
  const bytes = Buffer.from(JSON.stringify(empty));
  const signature = signIndex(bytes, keys.privatePem, 'test-1');
  const assets = [];
  const fetcher = async (url, options) => {
    const parsed = new URL(url);
    if (parsed.hostname === 'api.github.com') return new Response(JSON.stringify({assets}));
    assert.equal(options.headers['Cache-Control'], 'no-cache');
    if (!parsed.searchParams.get('_check')) return new Response('old cached alias');
    return new Response(parsed.pathname.endsWith('.sig') ? signature : bytes);
  };
  assert.equal((await readRegistry(config, fetcher)).recovered, false);
});


test('release metadata lookup by ID never depends on draft tag or list visibility', async () => {
  const draft = {id: 123, tag_name: 'loom.lang.json-0.1.0', draft: true, assets: []};
  const requests = [];
  const fetcher = async url => {
    requests.push(url);
    assert.equal(new URL(url).pathname, '/repos/acme/extensions/releases/123');
    return new Response(JSON.stringify(draft));
  };
  assert.deepEqual(await readRelease('acme/extensions', draft.tag_name, fetcher, {id: 123}), draft);
  assert.equal(requests.length, 1);
  assert.equal(await readRelease('acme/extensions', draft.tag_name, async () => new Response('', {status: 404}), {id: 123}), null);
  await assert.rejects(readRelease('acme/extensions', 'another-tag', fetcher, {id: 123}), /does not match/);
  await assert.rejects(readRelease('acme/extensions', draft.tag_name, async () => new Response('', {status: 403}), {id: 123}), /HTTP 403/);
});

test('new drafts publish by their creation ID even when tag and list discovery never see them', async t => {
  const ctx = publication(t), {item, prepared, plan} = await delivery(ctx);
  let discovery = 0, readbacks = 0;
  const lookup = async (repo, tag, fetcher, {id} = {}) => {
    if (id === undefined) {discovery++; return null;}
    readbacks++; const remote = ctx.remote.get(tag);
    assert.equal(remote.id, id); return remote;
  };
  const report = await finishRelease(plan, [prepared], {...ctx, lookup, privateKey: ctx.keys.privatePem});
  assert.deepEqual(report.failures, []);
  assert.deepEqual(report.indexed, [item.id]);
  assert.equal(discovery, 1); assert.equal(readbacks, 2);
  assert.deepEqual(ctx.calls, ['create', 'packages', 'publish', 'generation', 'index']);
});

test('post-write readback retries only verification and always requires complete matching bytes', async () => {
  const records = [{file:'package.loomext',size:1,sha256:'a'.repeat(64)}];
  const draft = {id:123,tag_name:'demo',draft:true,assets:[{name:'package.loomext',size:1,digest:'sha256:'+'a'.repeat(64)}]};
  const waits = []; let reads = 0;
  const lookup = async (repo, tag, fetcher, options) => {
    assert.deepEqual(options, {id:123});
    reads++; return reads===1 ? null : reads===2 ? {...draft,assets:[]} : draft;
  };
  assert.deepEqual(await verifyRelease('acme/extensions','demo',123,records,{lookup,wait:async ms=>waits.push(ms)}),draft);
  assert.equal(reads,3); assert.deepEqual(waits,[500,1500]);
  let checks=0;
  const published=await verifyRelease('acme/extensions','demo',123,records,{published:true,wait:async()=>{},lookup:async()=>({...draft,draft:++checks===1})});
  assert.equal(published.draft,false); assert.equal(checks,2);
  reads=0;
  await assert.rejects(verifyRelease('acme/extensions','demo',123,records,{wait:async()=>{},lookup:async()=>{reads++;return {...draft,assets:[{...draft.assets[0],digest:'sha256:'+'b'.repeat(64)}]};}}),/release 123.*SHA-256/);
  assert.equal(reads,3);
  await assert.rejects(verifyRelease('acme/extensions','demo',123,records,{lookup:async()=>{throw Error('HTTP 403');},wait:async()=>assert.fail('Permission errors must not be polled')}),/HTTP 403/);
});
