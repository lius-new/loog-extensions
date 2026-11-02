import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import {execFileSync} from 'node:child_process';
import {workspace, write} from './fixtures.mjs';
import {selectExtensions, changedFiles, run} from '../ci/select-extensions.mjs';

const ids = ['loom.lang.zig', 'loom.lang.c', 'loom.lsp.gopls'];
const sorted = [...ids].sort();

test('selection deduplicates affected existing extensions and ignores unrelated paths', () => {
  assert.deepEqual(selectExtensions(ids, [
    'extensions/loom.lang.zig/build.mjs', 'extensions/loom.lang.c/manifest.json',
    'extensions/loom.lang.c/deleted file.txt', 'extensions/loom.lang.removed/manifest.json',
    'docs/releases.md', 'README.md',
  ]), ['loom.lang.c', 'loom.lang.zig']);
  assert.deepEqual(selectExtensions(ids, ['README.md', 'docs/releases.md']), []);
  assert.deepEqual(selectExtensions(ids, []), []);
});

test('shared build and publication inputs select every extension', () => {
  for (const file of [
    '.github/workflows/extensions.yml', '.github/workflows/publish.yml',
    '.github/workflows/ext-loom.lang.removed.yml', 'scripts/ci/select-extensions.mjs',
    'scripts/lib/intake.mjs', 'tools/validator.lock', 'tools/validator-source.toml',
    'package.json', 'package-lock.json', 'loom-ext.toml',
  ]) assert.deepEqual(selectExtensions(ids, [file]), sorted, file);
  assert.deepEqual(selectExtensions(ids, [], {all: true}), sorted);
});

test('private language client changes rebuild all three official clients', () => {
  const clients = ['loom.lsp.rust-analyzer', 'loom.lsp.gopls', 'loom.lsp.typescript-language-server'];
  assert.deepEqual(selectExtensions([...ids, ...clients], ['language-clients/protocol.ts']), clients.sort());
});

test('manual selection is explicit, sorted and rejects unknown or malformed IDs', () => {
  assert.deepEqual(selectExtensions(ids, [], {all: true, requested: ' loom.lang.zig,loom.lang.c,loom.lang.c '}), ['loom.lang.c', 'loom.lang.zig']);
  assert.deepEqual(selectExtensions(ids, [], {all: true, requested: ' '}), sorted);
  for (const requested of ['unknown', '../loom.lang.c', 'loom.lang.c,', 'loom.lang.c,,loom.lang.zig']) {
    assert.throws(() => selectExtensions(ids, [], {requested}), /Unknown extension/);
  }
});

function repository(t) {
  const root = workspace(t);
  const git = (...args) => execFileSync('git', args, {cwd: root, encoding: 'utf8'}).trim();
  git('init', '-q');
  git('config', 'user.name', 'Selection test');
  git('config', 'user.email', 'selection@example.invalid');
  for (const id of ids) write(root, 'extensions/' + id + '/input.txt', 'initial');
  write(root, 'README.md', 'Documentation');
  const commit = () => {
    git('add', '.');
    git('commit', '-qm', 'fixture', '-m', 'Release Notes:\n- N/A');
    return git('rev-parse', 'HEAD');
  };
  const initial = commit();
  return {root, git, commit, initial};
}

test('push selection includes both ends of renames, deletions and paths containing newlines', t => {
  const {root, git, commit, initial} = repository(t);
  git('mv', 'extensions/loom.lang.c/input.txt', 'extensions/loom.lang.zig/moved file.txt');
  fs.rmSync(path.join(root, 'extensions/loom.lsp.gopls/input.txt'));
  // Newlines are valid Linux filenames; use a space on Windows.
  const unusual = process.platform === 'win32' ? 'a space.txt' : 'a\nnewline.txt';
  write(root, 'extensions/loom.lang.zig/' + unusual, 'new');
  commit();
  const changes = changedFiles('push', {before: initial}, root);
  assert.equal(changes.all, false);
  assert.ok(changes.files.includes('extensions/loom.lang.c/input.txt'));
  assert.ok(changes.files.includes('extensions/loom.lang.zig/moved file.txt'));
  assert.ok(changes.files.includes('extensions/loom.lsp.gopls/input.txt'));
  assert.ok(changes.files.includes('extensions/loom.lang.zig/' + unusual));
  assert.deepEqual(selectExtensions(ids, changes.files), sorted);
});

test('PR selection uses the merge base and excludes changes made only on the base branch', t => {
  const {root, git, commit, initial} = repository(t);
  git('checkout', '-qb', 'topic');
  write(root, 'extensions/loom.lang.c/input.txt', 'topic');
  commit();
  git('checkout', '-qb', 'base', initial);
  write(root, 'extensions/loom.lang.zig/input.txt', 'base branch only');
  const base = commit();
  git('checkout', '-q', 'topic');
  const changes = changedFiles('pull_request', {pull_request: {base: {sha: base}}}, root);
  assert.deepEqual(selectExtensions(ids, changes.files), ['loom.lang.c']);
  git('merge', '--no-edit', 'base');
  assert.deepEqual(selectExtensions(ids, changedFiles('pull_request', {pull_request: {base: {sha: base}}}, root).files), ['loom.lang.c']);
});

test('initial pushes, missing force-push tips and manual dispatch build everything', t => {
  const {root} = repository(t);
  for (const before of [undefined, '0'.repeat(40), 'a'.repeat(40)]) {
    assert.deepEqual(changedFiles('push', {before}, root), {all: true, files: []});
  }
  assert.deepEqual(changedFiles('workflow_dispatch', {}, root), {all: true, files: []});
  assert.throws(() => changedFiles('pull_request', {}, root), /comparison commit/);
  assert.throws(() => changedFiles('push', {before: '--help'}, root), /comparison commit/);
  assert.throws(() => changedFiles('workflow_run', {}, root), /Unsupported event/);
});

test('the planner writes a GitHub matrix and an explicit empty-batch guard', t => {
  const {root, commit, initial} = repository(t);
  const eventPath = write(root, 'event.json', {before: initial});
  const output = write(root, 'output.txt', '');
  write(root, 'README.md', 'Only documentation changed');
  commit();
  const env = {GITHUB_EVENT_PATH: eventPath, GITHUB_EVENT_NAME: 'push', GITHUB_OUTPUT: output,
    REQUESTED_EXTENSIONS: 'loom.lang.c'};
  assert.deepEqual(run(env, root), [], 'manual input cannot override push selection');
  assert.equal(fs.readFileSync(output, 'utf8'), 'matrix={"extension":[]}\nhas-extensions=false\n');
  fs.writeFileSync(output, '');
  assert.deepEqual(run({...env, GITHUB_EVENT_NAME: 'workflow_dispatch'}, root), ['loom.lang.c']);
  assert.equal(fs.readFileSync(output, 'utf8'), 'matrix={"extension":["loom.lang.c"]}\nhas-extensions=true\n');
});

test('deferred historical recipes are excluded from automatic and manual builds', t => {
  const {root} = repository(t);
  write(root, 'loom-ext.toml', 'schema = 1\nrepository = "acme/extensions"\nbundled = []\ndeferred = ["loom.lsp.gopls"]\n[index]\nkey-id = "test"\nvalidity-days = 30\n');
  const event = write(root, 'event.json', {});
  const output = write(root, 'output.txt', '');
  const env = {GITHUB_EVENT_PATH:event, GITHUB_OUTPUT:output, GITHUB_EVENT_NAME:'workflow_dispatch'};
  assert.deepEqual(run(env, root), ['loom.lang.c', 'loom.lang.zig']);
  assert.throws(() => run({...env, REQUESTED_EXTENSIONS:'loom.lsp.gopls'}, root), /Unknown extension/);
});
