import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import * as tar from 'tar';
import {assetUrl, checkRecipe, normalizeVersion, serverManifest, goBuildEnvironment} from '../lib/build-server.mjs';
import {hash} from '../lib/util.mjs';
import {checkout, cli, write} from './cli.mjs';

const recipe = ({extra = '', platforms = ['linux-x86_64']} = {}) => `schema = 1
kind = "server"

[extension]
name = "Demo LS"
version = "1.0.0"
description = "The Demo LS language server."
license = "MIT"

[pack]
kind = "archive"
version = "1.0.0"
upstream = { source = "github", repository = "acme/demo-ls", tag = "v1.0.0" }
${platforms.map(platform => `
[pack.platforms.${platform}]
asset = "demo-ls-${platform}.tar.gz"
format = "tar.gz"
executables = ["demo-ls"]
include = [{ from = "bin/demo-ls", to = "demo-ls" }]
`).join('')}
[servers.demo-ls]
name = "Demo LS"
command = "\${extension}/demo-ls"
args = ["--stdio"]
priority = 5
root = { markers = ["demo.toml"], strategy = "outermost", fallback = "workspace" }
languages = [{ language = "demo", language_id = "demo" }, { language = "other", variant = "x", language_id = "other-x" }]
${extra}`;

async function archive(root, name) {
  const source = path.join(root, 'src', 'demo-1.0');
  write(source, 'bin/demo-ls', 'binary contents');
  const file = path.join(root, name);
  await tar.c({file, cwd: path.join(root, 'src'), gzip: true}, ['demo-1.0']);
  return file;
}

async function demo(t, options = {}) {
  const repo = checkout(t, {bundled: []});
  const dir = path.join(repo.extensions, 'loom.lsp.demo');
  write(dir, 'recipe.toml', recipe(options));
  const platforms = options.platforms ?? ['linux-x86_64'];
  const work = path.join(repo.root, 'work');
  const pins = {schema: 1, platforms: {}};
  for (const platform of platforms) {
    const file = await archive(repo.root, `${platform}.tar.gz`);
    const sha256 = hash(fs.readFileSync(file));
    fs.mkdirSync(path.join(work, 'downloads'), {recursive: true});
    fs.copyFileSync(file, path.join(work, 'downloads', sha256));
    const url = assetUrl({version: '1.0.0', upstream: {source: 'github', repository: 'acme/demo-ls', tag: 'v1.0.0'}, platforms: {[platform]: {asset: `demo-ls-${platform}.tar.gz`}}}, platform);
    pins.platforms[platform] = {url, sha256, size: fs.statSync(file).size};
  }
  write(dir, 'pins.toml', Object.entries(pins.platforms).map(([platform, pin]) =>
    `[platforms.${platform}]\nurl = "${pin.url}"\nsha256 = "${pin.sha256}"\nsize = ${pin.size}\n`).join('\n').replace(/^/, 'schema = 1\n\n'));
  return {repo, dir, work};
}

test('versions written as dates become versions', () => {
  assert.equal(normalizeVersion('2026-10-09'), '2026.10.9');
  assert.equal(normalizeVersion('1.2.3'), '1.2.3');
});

test('a server recipe is checked for its pack, pins, commands and bindings', async t => {
  const { repo, dir } = await demo(t);
  assert.equal(cli(repo.root, ['check']).status, 0, cli(repo.root, ['check']).all);

  fs.rmSync(path.join(dir, 'pins.toml'));
  assert.match(cli(repo.root, ['check']).err, /missing pin for linux-x86_64; run: loom-ext pin loom\.lsp\.demo/);

  write(dir, 'recipe.toml', recipe().replace('command = "${extension}/demo-ls"', 'command = "demo-ls"').replace('markers = ["demo.toml"]', 'markers = []')
    .replace('fallback = "workspace"', 'fallback = "nowhere"').replace('tag = "v1.0.0"', 'tag = ""'));
  const text = cli(repo.root, ['check']).err;
  assert.match(text, /the command must start with/);
  assert.match(text, /invalid root markers/);
  assert.match(text, /invalid root fallback/);
  assert.match(text, /pin a GitHub repository and tag/);
});

test('a server needs the runtime it names in requires', async t => {
  const { repo, dir } = await demo(t);
  write(dir, 'recipe.toml', recipe().replace('command = "${extension}/demo-ls"', 'command = "${runtime.node}/node"'));
  assert.match(cli(repo.root, ['check']).err, /uses loom\.runtime\.node but the extension does not require it/);
  write(dir, 'recipe.toml', recipe().replace('license = "MIT"', 'license = "MIT"\nrequires = ["loom.runtime.node@^22"]').replace('command = "${extension}/demo-ls"', 'command = "${runtime.node}/node"'));
  assert.doesNotMatch(cli(repo.root, ['check']).err, /uses loom\.runtime/);
});

test('the extension version can advance independently of the pinned upstream program', async t => {
  const { repo, dir, work } = await demo(t);
  write(dir, 'recipe.toml', recipe().replace('version = "1.0.0"\ndescription', 'version = "2.0.0"\ndescription'));
  assert.equal(cli(repo.root, ['check']).status, 0);
  assert.equal(cli(repo.root, ['build', '--offline', '--work', work]).status, 0);
  const manifest = JSON.parse(fs.readFileSync(path.join(repo.root, 'dist/stage/linux-x86_64/loom.lsp.demo/manifest.json'), 'utf8'));
  assert.equal(manifest.version, '2.0.0');
});

test('a server builds from a pinned archive, once per platform, with no network', async t => {
  const platforms = ['linux-x86_64', 'windows-x86_64'];
  const { repo, work } = await demo(t, {platforms});
  const built = cli(repo.root, ['build', 'loom.lsp.demo', '--offline', '--work', work]);
  assert.equal(built.status, 0, built.all);
  assert.match(built.out, /Built loom\.lsp\.demo \(linux-x86_64, windows-x86_64\)/);

  const stage = platform => path.join(repo.root, 'dist/stage', platform, 'loom.lsp.demo');
  for (const platform of platforms) {
    assert.equal(fs.readFileSync(path.join(stage(platform), 'demo-ls'), 'utf8'), 'binary contents');
    for (const file of ['manifest.json', 'README.md', 'CHANGELOG.md', 'icon.svg', 'PROVENANCE.md']) assert.ok(fs.existsSync(path.join(stage(platform), file)), file);
  }
  const manifest = JSON.parse(fs.readFileSync(path.join(stage('linux-x86_64'), 'manifest.json'), 'utf8'));
  assert.equal(manifest.name, 'Demo LS Language Server');
  assert.deepEqual(manifest.capabilities, ['process.execute-local']);
  assert.deepEqual(manifest.executables, ['demo-ls']);
  assert.deepEqual(manifest.contributes.language_servers, [{
    id: 'loom.lsp.demo.demo-ls', name: 'Demo LS', languages: ['demo', 'other'], command: '${extension}/demo-ls', args: ['--stdio'],
    root_markers: ['demo.toml'], root_strategy: 'outermost', root_fallback: 'workspace', priority: 5,
    bindings: [{language: 'demo', language_id: 'demo'}, {language: 'other', variant: 'x', language_id: 'other-x'}],
  }]);

  const packed = cli(repo.root, ['pack', 'loom.lsp.demo']);
  assert.equal(packed.status, 0, packed.all);
  const listing = JSON.parse(fs.readFileSync(path.join(repo.root, 'dist/packages/packages.json'), 'utf8'));
  assert.deepEqual(listing.packages.map(record => record.platform), ['linux-x86_64', 'windows-x86_64']);
});

test('building one platform leaves the others to their own run, and a wrong platform is refused', async t => {
  const { repo, work } = await demo(t, {platforms: ['linux-x86_64', 'windows-x86_64']});
  assert.equal(cli(repo.root, ['build', 'loom.lsp.demo', '--offline', '--work', work, '--platforms', 'windows-x86_64']).status, 0);
  assert.ok(fs.existsSync(path.join(repo.root, 'dist/stage/windows-x86_64/loom.lsp.demo')));
  assert.ok(!fs.existsSync(path.join(repo.root, 'dist/stage/linux-x86_64/loom.lsp.demo')));
  assert.match(cli(repo.root, ['build', 'loom.lsp.demo', '--offline', '--work', work, '--platforms', 'macos-aarch64']).err, /not built for macos-aarch64/);
  assert.match(cli(repo.root, ['build', 'loom.lsp.demo', '--platforms', 'beos-ppc']).err, /Not a platform/);
});

test('a download that does not match its pin is refused offline', async t => {
  const { repo, work } = await demo(t);
  for (const name of fs.readdirSync(path.join(work, 'downloads'))) fs.writeFileSync(path.join(work, 'downloads', name), 'tampered');
  const built = cli(repo.root, ['build', 'loom.lsp.demo', '--offline', '--work', work]);
  assert.equal(built.status, 1);
  assert.match(built.err, /Missing or corrupt offline input/);
});

test('the declaration keeps what the application needs to start the server', () => {
  const manifest = serverManifest('loom.lsp.demo', {
    kind: 'server',
    extension: {name: 'Demo', version: '1.0.0', description: 'd', license: 'MIT', requires: ['loom.runtime.node@^22']},
    servers: {s: {name: 'S', command: '${runtime.node}/node', root: {markers: ['m'], fallback: 'none'}, languages: [{language: 'demo', language_id: 'demo'}]}},
  }, {repository: 'acme/x', executables: []});
  assert.deepEqual(manifest.requires, ['loom.runtime.node@^22']);
  assert.equal(manifest.contributes.language_servers[0].root_fallback, 'none');
  assert.ok(!('executables' in manifest));
});

test('the language servers and runtime in this repository have valid recipes', () => {
  const root = path.resolve(import.meta.dirname, '../../extensions');
  for (const id of fs.readdirSync(root).filter(name => name.startsWith('loom.lsp.') || name.startsWith('loom.runtime.'))) {
    const file = path.join(root, id, 'recipe.toml');
    assert.ok(fs.existsSync(file), id);
  }
  assert.equal(checkRecipe('loom.runtime.node', {kind: 'runtime', extension: {}, pack: {kind: 'cobol'}}).some(problem => /pack\.kind/.test(problem)), true);
});

test('Go builds isolate a pinned toolchain from inherited compiler roots and share verified module caches', () => {
  const parent = {GOROOT: 'old-go', GOTOOLDIR: 'old-compiler', GOBIN: 'global-bin', GOPROXY: 'https://proxy.golang.org', PATH: 'managed-go'};
  const pack = {go: '1.27.0', platforms: {'linux-aarch64': {goos: 'linux', goarch: 'arm64'}}};
  const env = goBuildEnvironment(pack, 'linux-aarch64', {work: path.resolve('work'), env: parent});
  assert.equal(env.GOTOOLCHAIN, 'go1.27.0'); assert.equal(env.GOOS, 'linux'); assert.equal(env.GOARCH, 'arm64');
  assert.equal(env.GOROOT, undefined); assert.equal(env.GOTOOLDIR, undefined); assert.equal(env.GOBIN, undefined);
  assert.equal(env.GOENV, 'off'); assert.equal(env.GOPROXY, parent.GOPROXY); assert.equal(env.PATH, parent.PATH);
  assert.equal(env.GOMODCACHE, path.resolve('work/go-cache/modules'));
  assert.equal(parent.GOROOT, 'old-go', 'the host environment stays unchanged');
});
