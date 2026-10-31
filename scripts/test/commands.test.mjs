import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import {checkout, cli, populate, write} from './cli.mjs';

test('check passes a good repository and names each problem in a bad one', t => {
  const repo = checkout(t);
  populate(repo);
  assert.equal(cli(repo.root, ['check']).status, 0);
  fs.rmSync(path.join(repo.extensions, 'loom.lang.demo/tree-sitter-demo.wasm'));
  const bad = cli(repo.root, ['check']);
  assert.equal(bad.status, 1);
  assert.match(bad.err, /loom\.lang\.demo:/);
  assert.match(bad.err, /missing WebAssembly module/);
});

test('check reports a bundled extension that does not exist and a file type claimed twice', t => {
  const repo = checkout(t, {bundled: ['loom.theme.demo', 'loom.lang.ghost']});
  populate(repo);
  assert.match(cli(repo.root, ['check']).err, /bundled extension loom\.lang\.ghost does not exist/);
  const clash = checkout(t, {bundled: []});
  populate(clash);
  const other = path.join(clash.extensions, 'loom.lang.other');
  fs.cpSync(path.join(clash.extensions, 'loom.lang.demo'), other, {recursive: true});
  write(other, 'manifest.json', {...JSON.parse(fs.readFileSync(path.join(other, 'manifest.json'), 'utf8')), id: 'loom.lang.other', contributes: {languages: [{id: 'loom.lang.other.grammar', path: 'tree-sitter.json'}]}});
  assert.match(cli(clash.root, ['check']).err, /file type demo is claimed by both/);
});

test('build and pack make one package per extension and record them', t => {
  const repo = checkout(t);
  populate(repo);
  assert.equal(cli(repo.root, ['build']).status, 0);
  const packed = cli(repo.root, ['pack']);
  assert.equal(packed.status, 0, packed.all);
  const listing = JSON.parse(fs.readFileSync(path.join(repo.root, 'dist/packages/packages.json'), 'utf8'));
  assert.deepEqual(listing.packages.map(record => `${record.id}@${record.version}:${record.platform}`),
    ['loom.lang.demo@1.0.0:any', 'loom.theme.demo@1.0.0:any']);
  for (const record of listing.packages) {
    assert.ok(fs.existsSync(path.join(repo.root, 'dist/packages', record.file)));
    assert.match(record.sha256, /^[a-f0-9]{64}$/);
  }
  // Packing again from the same stage is reproducible.
  const again = cli(repo.root, ['pack']);
  assert.equal(again.status, 0);
  assert.deepEqual(JSON.parse(fs.readFileSync(path.join(repo.root, 'dist/packages/packages.json'), 'utf8')), listing);
});

test('pack before build explains what is missing', t => {
  const repo = checkout(t);
  populate(repo, ['theme']);
  const result = cli(repo.root, ['pack']);
  assert.equal(result.status, 1);
  assert.match(result.err, /nothing is staged; run build first/);
});

test('dev copies the built extension into the folder Loom reads', t => {
  const repo = checkout(t);
  populate(repo);
  const target = path.join(repo.root, 'loom-home/extensions');
  assert.equal(cli(repo.root, ['dev', 'loom.theme.demo', '--to', target]).status, 0);
  assert.ok(fs.existsSync(path.join(target, 'loom.theme.demo/manifest.json')));
  assert.ok(!fs.existsSync(path.join(target, 'loom.lang.demo')));
  // The environment variable does the same, and a rebuild replaces the old copy.
  write(repo.extensions, 'loom.theme.demo/themes/dark.json', {name: 'Changed', base: 'aura-dark'});
  assert.equal(cli(repo.root, ['dev', 'loom.theme.demo'], {LOOM_EXTENSIONS_DIR: target}).status, 0);
  assert.match(fs.readFileSync(path.join(target, 'loom.theme.demo/themes/dark.json'), 'utf8'), /Changed/);
  assert.match(cli(repo.root, ['dev', 'loom.theme.demo']).err, /Say where to put them/);
});

test('unknown commands, options and extensions are refused', t => {
  const repo = checkout(t);
  populate(repo, ['theme']);
  assert.match(cli(repo.root, ['frobnicate']).err, /Unknown command/);
  assert.match(cli(repo.root, ['build', '--nope']).err, /Unknown option/);
  assert.match(cli(repo.root, ['check', 'loom.theme.ghost']).err, /Unknown extension/);
});
