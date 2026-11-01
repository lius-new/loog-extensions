import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import {execFileSync} from 'node:child_process';
import {createPackage} from '../lib/archive.mjs';
import {intake, platformOf} from '../lib/intake.mjs';
import {fingerprint} from '../lib/release.mjs';
import {language, theme, workspace, write} from './fixtures.mjs';

/** A repository holding extensions/<id>, and the packages of it built "elsewhere". */
function setup(t, {make = theme, id = 'loom.theme.demo', platforms = ['any']} = {}) {
  const root = workspace(t);
  make(path.join(root, 'extensions'), id);
  execFileSync('git', ['init', '-q'], {cwd: root});
  execFileSync('git', ['add', '-A'], {cwd: root});
  const packages = path.join(root, 'handed-over');
  return {root, dist: path.join(root, 'dist'), work: path.join(root, 'work'), id, packages, platforms,
    async build(version = '1.0.0', extra = {}) {
      const files = [];
      for (const platform of platforms) {
        const folder = path.join(root, 'out', platform, id);
        fs.rmSync(folder, {recursive: true, force: true});
        fs.cpSync(path.join(root, 'extensions', id), folder, {recursive: true});
        const manifest = JSON.parse(fs.readFileSync(path.join(folder, 'manifest.json'), 'utf8'));
        write(folder, 'manifest.json', {...manifest, version});
        for (const [file, content] of Object.entries(extra)) write(folder, file, content);
        const output = path.join(packages, `${id}-${version}-${platform}.loomext`);
        await createPackage(folder, output, id);
        files.push(output);
      }
      return files;
    }};
}

const accepts = () => ({validator: 'validator', validate: () => []});

test('packages are taken in by what they are, whatever project made them', async t => {
  const s = setup(t, {platforms: ['linux-x86_64', 'windows-x86_64']});
  const [result] = await intake(await s.build(), {...s, ...accepts()});
  assert.equal(result.id, s.id);
  assert.equal(result.version, '1.0.0');
  assert.deepEqual(result.records.map(record => record.platform), ['linux-x86_64', 'windows-x86_64']);
  assert.ok(result.records.every(record => fs.existsSync(path.join(s.dist, 'packages', record.file))));
  assert.equal(result.description.manifest.id, s.id);
  assert.equal(result.source.fingerprint, fingerprint(s.root, s.id));
});

test('a package Loom cannot import is rejected, and so is one named for something else', async t => {
  const s = setup(t, {make: language, id: 'loom.lang.demo'});
  const [file] = await s.build();
  const [bad] = await intake([file], {...s, validator: 'validator', validate: () => { throw new Error('grammar does not load'); }});
  assert.match(bad.error, /grammar does not load/);
  const renamed = path.join(s.packages, 'something-else.loomext');
  fs.copyFileSync(file, renamed);
  await assert.rejects(intake([renamed], {...s, ...accepts()}), /is named loom\.lang\.demo-1\.0\.0-<platform>\.loomext/);
  assert.throws(() => platformOf('x/loom.lang.demo-1.0.0-plan9.loomext', {id: 'loom.lang.demo', version: '1.0.0'}), /not a platform/);
});

test('a published version is skipped when unchanged, and refused when the extension changed under it', async t => {
  const s = setup(t);
  const files = await s.build();
  const index = fingerprints => ({extensions: [{id: s.id, versions: [{version: '1.0.0', source: {fingerprint: fingerprints}}]}]});
  const [skipped] = await intake(files, {...s, ...accepts(), index: index(fingerprint(s.root, s.id))});
  assert.match(skipped.skipped, /already published/);
  const [refused] = await intake(files, {...s, ...accepts(), index: index('0'.repeat(64))});
  assert.match(refused.error, /bump its version/);
});

test('a version released but never indexed keeps the bytes it was released with', async t => {
  const s = setup(t);
  const git = (...args) => execFileSync('git', ['-c', 'user.name=test', '-c', 'user.email=test@example.invalid', ...args], {cwd: s.root, encoding: 'utf8'}).trim();
  git('commit', '-q', '-m', 'released'); const released = git('rev-parse', 'HEAD');
  const original = await s.build();
  const [first] = await intake(original, {...s, ...accepts(), repository: 'acme/x', lookup: async () => null});
  const record = first.records[0], originalBytes = fs.readFileSync(original[0]);
  const remote = {draft: false, target_commitish: released, assets: [{name: record.file, size: record.size, digest: `sha256:${record.sha256}`, browser_download_url: 'https://example.invalid/package'}]};
  const download = async (url, sha, target) => fs.writeFileSync(target, originalBytes);
  const options = {...s, ...accepts(), repository: 'acme/x', lookup: async () => remote, download};

  // The same package again: nothing to take from the release.
  const [same] = await intake(await s.build(), options);
  assert.equal(same.recovered, true);
  assert.equal(same.records[0].sha256, record.sha256);

  // Built differently from the same sources: what was released is what is indexed.
  const [kept] = await intake(await s.build('1.0.0', {'rebuilt.txt': 'a different build'}), options);
  assert.equal(kept.error, undefined);
  assert.equal(kept.records[0].sha256, record.sha256);
  assert.deepEqual(fs.readFileSync(path.join(s.dist, 'packages', record.file)), originalBytes);

  // The sources changed since the release: that needs a new version.
  write(path.join(s.root, 'extensions', s.id), 'changed.txt', 'changed after the release');
  const [changed] = await intake(await s.build(), options);
  assert.match(changed.error, /differs from its released commit; bump its version/);
});

test('a new version has to be newer than the published ones', async t => {
  const s = setup(t);
  const files = await s.build('1.0.0');
  const index = {extensions: [{id: s.id, versions: [{version: '1.2.0', source: {fingerprint: 'f'.repeat(64)}}]}]};
  const [older] = await intake(files, {...s, ...accepts(), index});
  assert.match(older.error, /must be newer/);
  const [newer] = await intake(await s.build('1.3.0'), {...s, ...accepts(), index});
  assert.equal(newer.version, '1.3.0');
});
