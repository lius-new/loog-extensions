import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import {pathToFileURL} from 'node:url';
import {createPackage} from '../lib/archive.mjs';
import {hostPlatform} from '../lib/platform.mjs';
import {theme} from './fixtures.mjs';
import {checkout, cli, populate, write} from './cli.mjs';

/** Package extension `id` the way any project might, into `<root>/handed-over`. */
async function packageOf(repo, id, {version = '1.0.0', platform = 'any', change = () => {}} = {}) {
  const folder = path.join(repo.root, 'out', platform, id);
  fs.rmSync(folder, {recursive: true, force: true});
  fs.cpSync(path.join(repo.extensions, id), folder, {recursive: true});
  change(folder);
  const file = path.join(repo.root, 'handed-over', `${id}-${version}-${platform}.loomext`);
  await createPackage(folder, file, id);
  return file;
}

test('check passes good packages and names what is wrong with a bad one', async t => {
  const repo = checkout(t); populate(repo);
  await packageOf(repo, 'loom.theme.demo');
  const good = cli(repo.root, ['check', path.join(repo.root, 'handed-over')]);
  assert.equal(good.status, 0, good.all);
  assert.match(good.out, /Checked loom\.theme\.demo 1\.0\.0 \(any\)/);
  assert.match(good.err, /loom-ext-validate was not found/);
  await packageOf(repo, 'loom.lang.demo', {change: folder => fs.rmSync(path.join(folder, 'queries/highlights.scm'))});
  const bad = cli(repo.root, ['check', path.join(repo.root, 'handed-over')]);
  assert.notEqual(bad.status, 0);
  assert.match(bad.err, /loom\.lang\.demo-1\.0\.0-any\.loomext:[\s\S]*missing queries\/highlights\.scm/);
});

test('check refuses what is not a package of the name it carries', async t => {
  const repo = checkout(t); populate(repo, ['theme']);
  const file = await packageOf(repo, 'loom.theme.demo');
  const renamed = path.join(repo.root, 'handed-over', 'something-else.loomext');
  fs.copyFileSync(file, renamed);
  assert.match(cli(repo.root, ['check', renamed]).err, /is named loom\.theme\.demo-1\.0\.0-<platform>\.loomext/);
  assert.match(cli(repo.root, ['check']).err, /Name the \.loomext packages/);
});

test('dev unpacks the package for this machine into the folder Loom reads', async t => {
  const repo = checkout(t); populate(repo, ['theme']);
  await packageOf(repo, 'loom.theme.demo');
  const target = path.join(repo.root, 'loom-extensions-dir');
  write(target, 'loom.theme.demo/1.0.0/stale.txt', 'left from before');
  const result = cli(repo.root, ['dev', path.join(repo.root, 'handed-over'), '--to', target]);
  assert.equal(result.status, 0, result.all);
  const state = JSON.parse(fs.readFileSync(path.join(target, 'extensions.json'), 'utf8'));
  const entry = state.extensions['loom.theme.demo'];
  assert.equal(entry.version, '1.0.0');
  assert.equal(entry.pinned, true);
  assert.equal(entry.source.kind, 'file');
  assert.match(entry.source.sha256, /^[a-f0-9]{64}$/);
  const active = path.join(target, 'loom.theme.demo', entry.version);
  assert.ok(fs.existsSync(path.join(active, 'manifest.json')));
  assert.ok(fs.existsSync(path.join(active, 'themes/dark.json')));
  assert.ok(!fs.existsSync(path.join(active, 'stale.txt')), 'the version folder is replaced, not merged');
  assert.match(cli(repo.root, ['dev', path.join(repo.root, 'handed-over')], {LOOM_EXTENSIONS_DIR: ''}).err, /Say where to put them/);
});

/** Give extension `id` a build.mjs that packs it the way a project might, for the platform it is given. */
function buildable(repo, id) {
  const archive = pathToFileURL(path.join(import.meta.dirname, '../lib/archive.mjs')).href;
  write(repo.extensions, `${id}/build.mjs`, `import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {createPackage} from ${JSON.stringify(archive)};
const here = import.meta.dirname, id = path.basename(here);
const staged = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'try-build-')), id);
fs.cpSync(here, staged, {recursive: true, filter: source => !/(build\\.mjs|dist|seen\\.txt|source\\.txt)$/.test(source)});
fs.writeFileSync(path.join(here, 'seen.txt'), process.argv.slice(2).join(' '));
fs.writeFileSync(path.join(here, 'source.txt'), process.env.LOOM_SOURCE ?? '');
await createPackage(staged, path.join(here, 'dist', id + '-1.0.0-any.loomext'), id);
`);
}

test('try builds for this machine, unpacks, and starts Loom with the folder', async t => {
  const repo = checkout(t); populate(repo, ['theme']);
  buildable(repo, 'loom.theme.demo');
  write(repo.root, 'loom.mjs', `import fs from 'node:fs';
fs.writeFileSync('started.txt', process.env.LOOM_EXTENSIONS_DIR);
process.exit(3);
`);
  const target = path.join(repo.root, 'dev-dir');
  const result = cli(repo.root, ['try', 'loom.theme.demo', '--to', target, '--run', 'node loom.mjs']);
  assert.equal(result.status, 3, result.all);
  assert.equal(fs.readFileSync(path.join(repo.extensions, 'loom.theme.demo/seen.txt'), 'utf8'), hostPlatform());
  assert.ok(fs.existsSync(path.join(target, 'loom.theme.demo/1.0.0/manifest.json')));
  assert.equal(fs.readFileSync(path.join(repo.root, 'started.txt'), 'utf8'), target);
  assert.match(cli(repo.root, ['try', 'loom.theme.unknown', '--no-run']).err, /Unknown extension/);
  assert.match(cli(repo.root, ['try']).err, /Name the extensions to try/);
});

test('try takes the Loom checkout from --source, for the build and for running', async t => {
  const repo = checkout(t); populate(repo, ['theme']);
  buildable(repo, 'loom.theme.demo');
  const loom = path.join(repo.root, 'elsewhere/loom');
  assert.match(cli(repo.root, ['try', 'loom.theme.demo', '--source', loom, '--no-run']).err, /not a Loom checkout/);
  write(loom, 'Cargo.toml', '[package]\n');
  const result = cli(repo.root, ['try', 'loom.theme.demo', '--source', loom, '--to', path.join(repo.root, 'dev-dir'), '--run', 'node -e 1'], {LOOM_SOURCE: ''});
  assert.equal(result.status, 0, result.all);
  assert.equal(fs.readFileSync(path.join(repo.extensions, 'loom.theme.demo/source.txt'), 'utf8'), loom);
});

test('try removes the stale bundled extensions of the Loom checkout it runs with cargo', async t => {
  const repo = checkout(t); populate(repo, ['theme']);
  buildable(repo, 'loom.theme.demo');
  const loom = path.join(repo.root, 'elsewhere/loom');
  write(loom, 'Cargo.toml', '[package]\n');
  const bundled = path.join(loom, 'target/debug/extensions');
  write(bundled, 'loom.lang.json/manifest.json', '{}');
  const target = path.join(repo.root, 'dev-dir');
  // A command of your own runs elsewhere, and leaves the checkout's build output alone.
  const custom = cli(repo.root, ['try', 'loom.theme.demo', '--source', loom, '--to', target, '--run', 'node -e 1'], {LOOM_SOURCE: ''});
  assert.equal(custom.status, 0, custom.all);
  assert.ok(fs.existsSync(bundled));
  const bare = cli(repo.root, ['try', 'loom.theme.demo', '--source', loom, '--to', target, '--no-run'], {LOOM_SOURCE: ''});
  assert.equal(bare.status, 0, bare.all);
  assert.ok(fs.existsSync(bundled));
  // `cargo run` is not started in a test, so the cleanup it precedes is checked directly.
  const {dropStaleBundle} = await import('../commands/try.mjs');
  dropStaleBundle(loom);
  assert.ok(!fs.existsSync(bundled));
  assert.ok(fs.existsSync(path.join(loom, 'Cargo.toml')));
  dropStaleBundle(loom);
});

test('try builds what an extension requires unless it is already installed', async t => {
  const repo = checkout(t); populate(repo, ['theme']);
  theme(repo.extensions, 'loom.theme.base');
  for (const id of ['loom.theme.demo', 'loom.theme.base']) buildable(repo, id);
  const manifest = path.join(repo.extensions, 'loom.theme.demo/manifest.json');
  const demo = JSON.parse(fs.readFileSync(manifest, 'utf8'));
  write(repo.extensions, 'loom.theme.demo/manifest.json', {...demo, requires: ['loom.theme.base@^1.0.0']});
  const target = path.join(repo.root, 'dev-dir');
  const first = cli(repo.root, ['try', 'loom.theme.demo', '--to', target, '--no-run']);
  assert.equal(first.status, 0, first.all);
  assert.match(first.out, /requires loom\.theme\.base/);
  assert.ok(fs.existsSync(path.join(target, 'loom.theme.base/1.0.0/manifest.json')));
  const second = cli(repo.root, ['try', 'loom.theme.demo', '--to', target, '--no-run']);
  assert.doesNotMatch(second.out, /requires loom\.theme\.base/);
});

test('unknown commands and the removed build commands are refused', t => {
  const repo = checkout(t);
  assert.match(cli(repo.root, ['frobnicate']).err, /Unknown command/);
  for (const command of ['build', 'pack', 'index', 'publish', 'validate', 'pin']) assert.match(cli(repo.root, [command]).err, /Unknown command/, command);
  assert.equal(cli(repo.root, ['help']).status, 0);
});
