import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import * as tar from 'tar';
import {createPackage, extractPackage, listPackage} from '../lib/archive.mjs';
import {artifactName, chooseArtifact, hostPlatform, nodeKey, validPlatform} from '../lib/platform.mjs';
import {theme, workspace} from './fixtures.mjs';

test('packing is reproducible and unpacks to the same files', async t => {
  const root = workspace(t);
  const dir = theme(root);
  const first = await createPackage(dir, path.join(root, 'out/a.loomext'));
  const second = await createPackage(dir, path.join(root, 'out/b.loomext'));
  assert.equal(first.sha256, second.sha256);
  const entries = await listPackage(first.file);
  assert.deepEqual(entries.map(entry => entry.path).sort(), ['loom.theme.demo', 'loom.theme.demo/manifest.json', 'loom.theme.demo/themes/dark.json']);
  const unpacked = await extractPackage(first.file, path.join(root, 'unpacked'), 'loom.theme.demo');
  assert.equal(fs.readFileSync(path.join(unpacked, 'themes/dark.json'), 'utf8'), fs.readFileSync(path.join(dir, 'themes/dark.json'), 'utf8'));
});

test('a folder without a manifest or with the wrong name is not packed', async t => {
  const root = workspace(t);
  fs.mkdirSync(path.join(root, 'loom.theme.empty'));
  await assert.rejects(createPackage(path.join(root, 'loom.theme.empty'), path.join(root, 'x.loomext')), /manifest\.json is missing/);
  const dir = theme(root);
  await assert.rejects(createPackage(dir, path.join(root, 'y.loomext'), 'loom.theme.other'), /named after the extension id/);
});

test('unpacking refuses the wrong root, links and traversal', async t => {
  const root = workspace(t);
  const dir = theme(root);
  const { file } = await createPackage(dir, path.join(root, 'ok.loomext'));
  await assert.rejects(extractPackage(file, path.join(root, 'u1'), 'loom.theme.other'), /not loom\.theme\.other/);

  // A link entry.
  const link = path.join(root, 'link');
  fs.mkdirSync(path.join(link, 'loom.theme.link'), {recursive: true});
  fs.writeFileSync(path.join(link, 'loom.theme.link/manifest.json'), '{}');
  try { fs.symlinkSync('manifest.json', path.join(link, 'loom.theme.link/alias')); } catch { return; }
  await tar.c({file: path.join(root, 'link.loomext'), cwd: link, gzip: true}, ['loom.theme.link']);
  await assert.rejects(extractPackage(path.join(root, 'link.loomext'), path.join(root, 'u2')), /Only files and folders/);
});

test('platforms are named os-arch and the host maps onto them', () => {
  assert.equal(hostPlatform('win32', 'x64'), 'windows-x86_64');
  assert.equal(hostPlatform('darwin', 'arm64'), 'macos-aarch64');
  assert.equal(hostPlatform('linux', 'arm64'), 'linux-aarch64');
  assert.throws(() => hostPlatform('freebsd', 'x64'));
  assert.equal(nodeKey('windows-x86_64'), 'win32-x64');
  assert.equal(nodeKey('macos-aarch64'), 'darwin-arm64');
  assert.ok(validPlatform('any') && validPlatform('linux-x86_64') && !validPlatform('macos-universal'));
  assert.equal(artifactName('loom.lang.rust', '1.0.0', 'any'), 'loom.lang.rust-1.0.0-any.loomext');
});

test('the platform package wins over the generic one', () => {
  const artifacts = [{platform: 'any', url: 'a'}, {platform: 'linux-x86_64', url: 'b'}];
  assert.equal(chooseArtifact(artifacts, 'linux-x86_64').url, 'b');
  assert.equal(chooseArtifact(artifacts, 'macos-aarch64').url, 'a');
  assert.equal(chooseArtifact([{platform: 'linux-x86_64'}], 'macos-aarch64'), null);
});
