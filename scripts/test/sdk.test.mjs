import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import {prepareSdk} from '../lib/sdk.mjs';
import {workspace, write} from './fixtures.mjs';

function source(t) {
  const root = workspace(t);
  write(root, 'wit/loog-extension/world.wit', 'package loog:extension;');
  write(root, 'crates/workspace-api/Cargo.toml', '[package]\nname = "loog-workspace-api"');
  return root;
}

test('WIT and Rust dependencies use one existing SDK without copying sources', t => {
  const sdk = source(t), root = workspace(t);
  const directory = prepareSdk(root, sdk);
  assert.equal(fs.realpathSync.native(directory), fs.realpathSync.native(sdk));
  assert.equal(prepareSdk(root, sdk), directory);
  assert.equal(prepareSdk(root), directory);
  const other = source(t);
  assert.throws(() => prepareSdk(root, other), /different SDK/);
  assert.equal(fs.realpathSync.native(directory), fs.realpathSync.native(sdk));
});

test('CI checkouts are reused and missing workspace SDKs are refused', t => {
  const root = workspace(t), directory = path.join(root, '.work/loog-sdk');
  write(directory, 'wit/loog-extension/world.wit', 'package loog:extension;');
  assert.throws(() => prepareSdk(root), /workspace support/);
  write(directory, 'crates/workspace-api/Cargo.toml', '[package]');
  assert.equal(prepareSdk(root), directory);
  assert.equal(fs.lstatSync(directory).isSymbolicLink(), false);
});
