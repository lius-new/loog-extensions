import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {developmentDirectory, registerDevelopment} from '../lib/development.mjs';

function fixture(t) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'loog-development-'));
  t.after(() => fs.rmSync(root, {recursive: true, force: true}));
  return root;
}
function stage(root, id, version) {
  const directory = developmentDirectory(root, {id, version});
  fs.mkdirSync(directory, {recursive: true});
  fs.writeFileSync(path.join(directory, 'manifest.json'), JSON.stringify({id, version}));
  fs.writeFileSync(path.join(directory, 'extension.wasm'), 'new component');
  return {directory};
}
test('development registration replaces stale active versions and preserves unrelated choices', t => {
  const root = fixture(t);
  const other = {version: '1.0.0', source: {kind: 'file', sha256: '0'.repeat(64)}};
  fs.writeFileSync(path.join(root, 'extensions.json'), JSON.stringify({extensions: {
    'loog.lsp.demo': {...other, version: '0.2.0'}, 'loog.lang.demo': other,
  }, disabled: ['loog.lang.demo']}));
  registerDevelopment(root, [stage(root, 'loog.lsp.demo', '0.2.2')]);
  const state = JSON.parse(fs.readFileSync(path.join(root, 'extensions.json'), 'utf8'));
  const entry = state.extensions['loog.lsp.demo'];
  assert.equal(entry.version, '0.2.2');
  assert.equal(entry.previous, '0.2.0');
  assert.equal(entry.pinned, true);
  assert.match(entry.source.sha256, /^[a-f0-9]{64}$/);
  assert.deepEqual(state.extensions['loog.lang.demo'], other);
  assert.deepEqual(state.disabled, ['loog.lang.demo']);
  const discovered = path.join(root, 'loog.lsp.demo', entry.version, 'manifest.json');
  assert.equal(JSON.parse(fs.readFileSync(discovered, 'utf8')).version, '0.2.2');
});
test('a failed batch leaves the activation index unchanged', t => {
  const root = fixture(t);
  const first = stage(root, 'loog.first', '1.0.0');
  registerDevelopment(root, [first]);
  const before = fs.readFileSync(path.join(root, 'extensions.json'), 'utf8');
  const second = stage(root, 'loog.second', '1.0.0');
  assert.throws(() => registerDevelopment(root, [second, {directory: path.join(root, 'missing')}]), /ENOENT/);
  assert.equal(fs.readFileSync(path.join(root, 'extensions.json'), 'utf8'), before);
});
test('changed component bytes update a same-version development fingerprint', t => {
  const root = fixture(t), item = stage(root, 'loog.demo', '1.0.0');
  registerDevelopment(root, [item]);
  const entry = () => JSON.parse(fs.readFileSync(path.join(root, 'extensions.json'), 'utf8')).extensions['loog.demo'];
  const before = entry().source.sha256;
  fs.writeFileSync(path.join(item.directory, 'extension.wasm'), 'changed component');
  registerDevelopment(root, [item]);
  assert.notEqual(entry().source.sha256, before);
  assert.equal(entry().version, '1.0.0');
});
test('development paths reject escaping identifiers and versions', () => {
  assert.throws(() => developmentDirectory('staging', {id: '../other', version: '1.0.0'}), /Invalid/);
  assert.throws(() => developmentDirectory('staging', {id: 'loog.demo', version: '../other'}), /Invalid/);
});

test('a source directory cannot accidentally become a serialized Source enum', t => {
  const root = fixture(t), item = stage(root, 'loog.demo', '1.0.0');
  registerDevelopment(root, [item]);
  const stateFile = path.join(root, 'extensions.json');
  const before = fs.readFileSync(stateFile, 'utf8');
  assert.throws(() => registerDevelopment(root, [{...item, source: '/checkout/extensions/loog.demo'}]), /tagged source record/);
  assert.equal(fs.readFileSync(stateFile, 'utf8'), before);
  assert.equal(typeof JSON.parse(before).extensions['loog.demo'].source, 'object');
});

test('an invalid retained entry cannot silently corrupt a new activation index', t => {
  const root = fixture(t), item = stage(root, 'loog.demo', '1.0.0');
  const stateFile = path.join(root, 'extensions.json');
  const before = JSON.stringify({extensions: {'loog.other': {version: '1.0.0', source: '/wrong/path'}}, disabled: []});
  fs.writeFileSync(stateFile, before);
  assert.throws(() => registerDevelopment(root, [item]), /Invalid activation record for loog.other/);
  assert.equal(fs.readFileSync(stateFile, 'utf8'), before);
});
