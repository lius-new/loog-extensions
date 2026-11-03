import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import {createPackage} from '../lib/archive.mjs';
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

test('unknown commands and the removed build commands are refused', t => {
  const repo = checkout(t);
  assert.match(cli(repo.root, ['frobnicate']).err, /Unknown command/);
  for (const command of ['build', 'pack', 'index', 'publish', 'validate', 'pin']) assert.match(cli(repo.root, [command]).err, /Unknown command/, command);
  assert.equal(cli(repo.root, ['help']).status, 0);
});
