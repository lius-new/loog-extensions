// Builds language server and runtime extensions. The program comes from an upstream
// release archive (repackaged unchanged), from npm (locked, installed without
// scripts) or from a Go module (compiled for each platform). Every download is pinned
// by hash before it is trusted.
import fs from 'node:fs';
import path from 'node:path';
import zlib from 'node:zlib';
import {DIST, EXTENSIONS, HASH, WORK, assert, inside, readToml, relative, removeUnder, run} from './util.mjs';
import {extractInto, FORMATS} from './unpack.mjs';
import {fetchPinned} from './fetch.mjs';
import {checkExtension, parseRequirement, validId} from './manifest.mjs';
import {validVersion} from './semver.mjs';
import {ANY, PLATFORMS} from './platform.mjs';
import {loadConfig} from './config.mjs';
import {stageDirectory, stageRoot} from './stage.mjs';
import {serverDocuments} from './documents.mjs';

const LANGUAGE = /^[a-z][a-z0-9_]*$/;
const FILE_NAME = /^[A-Za-z0-9][A-Za-z0-9._-]*$/;
const EXACT_PACKAGE = /^(@[\w.-]+\/)?[\w.-]+@\d+\.\d+\.\d+$/;
const SERVER_ID = /^[a-z][a-z0-9_-]*$/;
const placeholder = /^\$\{(extension|runtime\.[a-z][a-z0-9-]*)\}\//;

export const pinsFile = (id, root = EXTENSIONS) => path.join(root, id, 'pins.toml');
export const readPins = (id, root = EXTENSIONS) => (fs.existsSync(pinsFile(id, root)) ? readToml(pinsFile(id, root)) : {schema: 1});

/** Where an upstream archive for `platform` is downloaded from. */
export function assetUrl(pack, platform) {
  const spec = pack.platforms[platform], upstream = pack.upstream;
  if (upstream.source === 'nodejs') return `https://nodejs.org/dist/v${pack.version}/${spec.asset}`;
  return `https://github.com/${upstream.repository}/releases/download/${encodeURIComponent(upstream.tag)}/${encodeURIComponent(spec.asset)}`;
}

/** `2026-10-19` is written `2026.10.19` in versions; everything else is already a version. */
export const normalizeVersion = text => (/^\d{4}-\d{2}-\d{2}$/.test(text) ? text.split('-').map(Number).join('.') : text);

/** The platforms the extension is built for. */
export const packPlatforms = pack => (pack.kind === 'npm' ? [ANY] : Object.keys(pack.platforms ?? {}));

function checkServer(id, server, recipe, problems) {
  const bad = message => problems.push(`server ${id}: ${message}`);
  if (!SERVER_ID.test(id)) bad('invalid id');
  if (typeof server.name !== 'string' || !server.name.trim() || server.name !== server.name.trim()) bad('missing name');
  const match = placeholder.exec(server.command ?? '');
  if (!match) bad('the command must start with ${extension}/ or ${runtime.<id>}/ and never names a program found on PATH');
  else if (match[1] !== 'extension') {
    const runtime = `loom.runtime.${match[1].slice('runtime.'.length)}`;
    if (!(recipe.extension.requires ?? []).some(item => item.split('@')[0] === runtime)) bad(`it uses ${runtime} but the extension does not require it`);
  }
  if (/[\0-\x1f]/.test(server.command ?? '')) bad('invalid command');
  for (const arg of server.args ?? []) if (typeof arg !== 'string' || /[\0-\x1f]/.test(arg)) bad('invalid argument');
  if (server.priority !== undefined && !Number.isSafeInteger(server.priority)) bad('invalid priority');
  const options = server.initialization_options;
  if (options !== undefined && (!options || typeof options !== 'object' || Array.isArray(options))) bad('initialization_options must be an object');
  const root = server.root;
  if (!root || !Array.isArray(root.markers) || !root.markers.length || !root.markers.every(m => typeof m === 'string' && m.trim() && !/[\0-\x1f/\\]/.test(m))) bad('invalid root markers');
  if (root && root.strategy !== undefined && !['nearest', 'outermost'].includes(root.strategy)) bad('invalid root strategy');
  if (root && !['workspace', 'file_directory', 'none'].includes(root.fallback)) bad('invalid root fallback');
  if (!Array.isArray(server.languages) || !server.languages.length) { bad('it serves at least one language'); return; }
  const seen = new Set();
  for (const binding of server.languages) {
    if (!LANGUAGE.test(binding.language ?? '') || (binding.variant !== undefined && !LANGUAGE.test(binding.variant))) bad('invalid language binding');
    if (typeof binding.language_id !== 'string' || !binding.language_id || /[\s\0-\x1f]/.test(binding.language_id)) bad('invalid language_id');
    const key = `${binding.language}/${binding.variant ?? ''}`;
    if (seen.has(key)) bad(`language bound twice: ${key}`);
    seen.add(key);
  }
}

/** Problems with a server or runtime recipe that need no network. */
export function checkRecipe(id, recipe, {root = EXTENSIONS} = {}) {
  const problems = [];
  const bad = message => problems.push(message);
  const runtime = recipe.kind === 'runtime';
  if (!id.startsWith(runtime ? 'loom.runtime.' : 'loom.lsp.') || !validId(id)) bad(`a ${recipe.kind} extension is named ${runtime ? 'loom.runtime.<name>' : 'loom.lsp.<name>'}`);
  const extension = recipe.extension ?? {};
  if (typeof extension.name !== 'string' || !extension.name.trim() || extension.name.length > 50) bad('extension.name is required (up to 50 characters)');
  if (!validVersion(extension.version ?? '')) bad('extension.version must be a version such as 1.0.0');
  if (typeof extension.description !== 'string' || !extension.description.trim() || extension.description.length > 200) bad('extension.description is required (up to 200 characters)');
  if (typeof extension.license !== 'string' || !extension.license.trim() || extension.license.length > 100) bad('extension.license is required (an SPDX expression)');
  for (const item of extension.requires ?? []) if (!parseRequirement(item)) bad(`extension.requires: ${JSON.stringify(item)} is not id@range`);
  const pack = recipe.pack ?? {};
  if (!['archive', 'npm', 'go'].includes(pack.kind)) { bad(`pack.kind must be archive, npm or go`); return problems; }
  if (typeof pack.version !== 'string' || !/^[\w.+-]+$/.test(pack.version)) bad('pack.version must be pinned');
  else if (normalizeVersion(pack.version) !== extension.version) bad(`extension.version (${extension.version}) must be the pack version (${normalizeVersion(pack.version)})`);
  const names = new Set();
  for (const license of pack.licenses ?? []) {
    let https = false;
    try { https = new URL(license.url).protocol === 'https:'; } catch { /* reported below */ }
    if (pack.kind === 'npm') bad('an npm pack carries its licenses in node_modules');
    else if (!FILE_NAME.test(license.name ?? '') || names.has(license.name) || !https) bad(`invalid license reference ${license.name}`);
    names.add(license.name);
  }
  if (pack.kind === 'go' && !pack.licenses?.length) bad('a Go pack names its license files');
  if (pack.kind === 'npm') {
    if (pack.platforms !== undefined) bad('an npm pack is built once for every platform');
    if (!Array.isArray(pack.packages) || !pack.packages.length || !pack.packages.every(item => EXACT_PACKAGE.test(item))) bad('pin exact npm packages');
    else problems.push(...checkLock(id, pack, root));
  } else {
    if (!pack.platforms || !Object.keys(pack.platforms).length) bad('a pack needs at least one platform');
    for (const [platform, spec] of Object.entries(pack.platforms ?? {})) {
      const here = message => bad(`${platform}: ${message}`);
      if (!PLATFORMS.includes(platform)) here('not a platform');
      if (!Array.isArray(spec.executables) || !spec.executables.length) here('name the executables');
      else for (const file of spec.executables) { try { relative(file, 'executable'); } catch (error) { here(error.message); } }
      if (pack.kind === 'archive') {
        if (typeof spec.asset !== 'string' || /[\\/\0]/.test(spec.asset)) here('invalid asset');
        if (!FORMATS.includes(spec.format)) here('unknown format');
        if (['gz', 'raw'].includes(spec.format) && (spec.executables?.length !== 1 || spec.include)) here('a single-file asset names one executable and includes nothing');
        for (const item of spec.include ?? []) {
          try { relative(typeof item === 'string' ? item : item.from, 'include'); relative(typeof item === 'string' ? item : item.to, 'include'); } catch (error) { here(error.message); }
        }
      } else if (!/^[a-z0-9]+$/.test(spec.goos ?? '') || !/^[a-z0-9]+$/.test(spec.goarch ?? '')) here('invalid Go target');
    }
    if (pack.kind === 'archive') {
      const upstream = pack.upstream ?? {};
      if (!['nodejs', 'github'].includes(upstream.source)) bad('unknown upstream source');
      else if (upstream.source === 'github' && (!/^[\w.-]+\/[\w.-]+$/.test(upstream.repository ?? '') || !upstream.tag)) bad('pin a GitHub repository and tag');
      else problems.push(...checkPins(id, pack, root));
    } else if (!/^\d+\.\d+\.\d+$/.test(pack.go ?? '') || !/^v\d+\.\d+\.\d+$/.test(pack['module-version'] ?? '') || !/^[\w./-]+$/.test(pack.module ?? '')) bad('pin the Go toolchain and module');
    else problems.push(...checkPins(id, pack, root));
  }
  const servers = recipe.servers ?? {};
  if (runtime && Object.keys(servers).length) bad('a runtime declares no servers');
  if (!runtime && !Object.keys(servers).length) bad('declare at least one server');
  for (const [serverId, server] of Object.entries(servers)) checkServer(serverId, server, recipe, problems);
  return problems;
}

/** Every archive and license download needs a pin, and a pin needs a download. */
function checkPins(id, pack, root) {
  const problems = [];
  const pins = readPins(id, root);
  const wanted = new Set();
  if (pack.kind === 'archive') {
    for (const platform of Object.keys(pack.platforms)) {
      const pin = pins.platforms?.[platform];
      if (!pin) problems.push(`missing pin for ${platform}; run: loom-ext pin ${id}`);
      else if (pin.url !== assetUrl(pack, platform)) problems.push(`stale pin for ${platform}; run: loom-ext pin ${id}`);
      else if (!HASH.test(pin.sha256 ?? '') || !Number.isSafeInteger(pin.size) || pin.size <= 0) problems.push(`invalid pin for ${platform}`);
      wanted.add(`platforms.${platform}`);
    }
  }
  for (const license of pack.licenses ?? []) {
    const pin = pins.licenses?.[license.name];
    if (!pin || pin.url !== license.url || !HASH.test(pin.sha256 ?? '')) problems.push(`missing or stale pin for ${license.name}; run: loom-ext pin ${id}`);
    wanted.add(`licenses.${license.name}`);
  }
  for (const platform of Object.keys(pins.platforms ?? {})) if (!wanted.has(`platforms.${platform}`)) problems.push(`pin for no platform: ${platform}`);
  for (const name of Object.keys(pins.licenses ?? {})) if (!wanted.has(`licenses.${name}`)) problems.push(`pin for no license: ${name}`);
  return problems;
}

/** Lockfiles pin every npm dependency by integrity hash. */
function checkLock(id, pack, root) {
  const problems = [];
  const directory = path.join(root, id, 'npm');
  const manifest = path.join(directory, 'package.json'), lock = path.join(directory, 'package-lock.json');
  if (!fs.existsSync(manifest) || !fs.existsSync(lock)) return [`missing npm lockfile; run: loom-ext pin ${id}`];
  let dependencies, locked;
  try { dependencies = JSON.parse(fs.readFileSync(manifest, 'utf8')).dependencies ?? {}; locked = JSON.parse(fs.readFileSync(lock, 'utf8')); }
  catch (error) { return [`npm lockfile: ${error.message}`]; }
  if (!(locked.lockfileVersion >= 2) || !locked.packages) return ['unsupported npm lockfile'];
  for (const spec of pack.packages) {
    const at = spec.lastIndexOf('@'), name = spec.slice(0, at), version = spec.slice(at + 1);
    if (dependencies[name] !== version) problems.push(`package.json does not pin ${spec}; run: loom-ext pin ${id}`);
    if (locked.packages[`node_modules/${name}`]?.version !== version) problems.push(`the lockfile does not pin ${spec}; run: loom-ext pin ${id}`);
  }
  for (const [name, entry] of Object.entries(locked.packages)) {
    if (name === '' || entry.link) continue;
    if (!/^sha(256|384|512)-/.test(entry.integrity ?? '')) problems.push(`${name} has no integrity hash in the lockfile`);
  }
  return problems;
}

// ------------------------------------------------------------------ assembly

const npmCommand = process.platform === 'win32' ? 'npm.cmd' : 'npm';

function copyInclude(root, item, directory) {
  const from = typeof item === 'string' ? item : item.from, to = typeof item === 'string' ? item : item.to;
  const source = inside(root, from), target = path.join(directory, relative(to, 'include'));
  fs.mkdirSync(path.dirname(target), {recursive: true});
  fs.copyFileSync(source, target);
}

async function assembleArchive(id, pack, pins, platform, directory, {work, offline}) {
  const spec = pack.platforms[platform], pin = pins.platforms[platform];
  const archive = await fetchPinned(pin.url, pin.sha256, path.join(work, 'downloads', pin.sha256), {offline});
  if (spec.format === 'gz') fs.writeFileSync(path.join(directory, spec.executables[0]), zlib.gunzipSync(fs.readFileSync(archive)));
  else if (spec.format === 'raw') fs.copyFileSync(archive, path.join(directory, spec.executables[0]));
  else {
    const base = path.join(work, 'extracted', pin.sha256), content = path.join(base, 'content');
    if (!fs.existsSync(path.join(base, 'complete'))) {
      removeUnder(work, base);
      await extractInto(archive, spec.format, content);
      fs.writeFileSync(path.join(base, 'complete'), pin.sha256);
    }
    let root = content;
    const entries = fs.readdirSync(root);
    if (entries.length === 1 && fs.statSync(path.join(root, entries[0])).isDirectory()) root = path.join(root, entries[0]);
    if (spec.include) for (const item of spec.include) copyInclude(root, item, directory);
    else fs.cpSync(root, directory, {recursive: true});
  }
  return `Upstream: ${pin.url}\nSHA-256: ${pin.sha256}\n`;
}

function assembleNpm(id, directory, {work, root}) {
  const source = path.join(root, id, 'npm'), build = path.join(work, 'npm-build', id);
  removeUnder(work, build);
  fs.mkdirSync(build, {recursive: true});
  for (const name of ['package.json', 'package-lock.json']) fs.copyFileSync(path.join(source, name), path.join(build, name));
  // Lifecycle scripts never run, and the lockfile is the only source of versions.
  run(npmCommand, ['ci', '--ignore-scripts', '--omit=dev', '--no-audit', '--no-fund', '--no-bin-links'],
    {cwd: build, env: {...process.env, npm_config_update_notifier: 'false'}, shell: process.platform === 'win32'});
  fs.cpSync(path.join(build, 'node_modules'), path.join(directory, 'node_modules'), {recursive: true});
  const locked = JSON.parse(fs.readFileSync(path.join(build, 'package-lock.json'), 'utf8')).packages;
  const list = Object.entries(locked).filter(([name, entry]) => name && !entry.link)
    .map(([name, entry]) => `- ${name.replace(/^node_modules\//, '')} ${entry.version} ${entry.integrity}`).sort();
  return `Packages (${list.length}), locked by integrity:\n${list.join('\n')}\n`;
}

function assembleGo(pack, platform, directory, {work}) {
  const spec = pack.platforms[platform];
  const gopath = path.join(work, 'gopath', platform), module = `${pack.module}@${pack['module-version']}`;
  fs.mkdirSync(work, {recursive: true});
  const env = {...process.env, GOPATH: gopath, GOOS: spec.goos, GOARCH: spec.goarch, CGO_ENABLED: '0', GOTOOLCHAIN: `go${pack.go}`, GOFLAGS: '-trimpath', GOWORK: 'off'};
  delete env.GOBIN;
  run('go', ['install', module], {env, cwd: work});
  // A cross-compiled binary lands in a per-target folder.
  const found = spec.executables.map(name => [path.join(gopath, 'bin', `${spec.goos}_${spec.goarch}`, name), path.join(gopath, 'bin', name)].find(file => fs.existsSync(file)));
  assert(found.every(Boolean), `go install did not produce ${spec.executables.join(', ')}`);
  spec.executables.forEach((name, index) => fs.copyFileSync(found[index], path.join(directory, name)));
  const info = JSON.parse(run('go', ['mod', 'download', '-json', module], {env: {...env, GOOS: undefined, GOARCH: undefined}, cwd: work}));
  return `Module: ${module}\nModule sum: ${info.Sum}\nGo: ${pack.go}\nTarget: ${spec.goos}/${spec.goarch}\n`;
}

/** `${pack}` in an old recipe is `${extension}` here; the manifest keeps the placeholders as written. */
export function serverDeclaration(extensionId, serverId, server) {
  const declaration = {
    id: `${extensionId}.${serverId}`, name: server.name,
    languages: [...new Set(server.languages.map(binding => binding.language))],
    command: server.command, ...(server.args?.length ? {args: server.args} : {}),
    root_markers: server.root.markers,
    ...(server.root.strategy ? {root_strategy: server.root.strategy} : {}),
    root_fallback: server.root.fallback,
    ...(server.priority ? {priority: server.priority} : {}),
    bindings: server.languages.map(binding => ({language: binding.language, ...(binding.variant ? {variant: binding.variant} : {}), language_id: binding.language_id})),
    ...(server.initialization_options ? {initialization_options: server.initialization_options} : {}),
  };
  return declaration;
}

export function serverManifest(id, recipe, {repository, executables}) {
  const extension = recipe.extension, runtime = recipe.kind === 'runtime';
  const servers = Object.entries(recipe.servers ?? {});
  return {
    schema: 1, id, version: extension.version, name: runtime ? `${extension.name} Runtime` : `${extension.name} Language Server`,
    publisher: 'Loom', description: extension.description, categories: [runtime ? 'Other' : 'Languages'],
    keywords: runtime ? [extension.name.toLowerCase(), 'runtime'] : [extension.name.toLowerCase(), 'language server', 'lsp'],
    license: extension.license, repository: `https://github.com/${repository}`, icon: 'icon.svg', readme: 'README.md', changelog: 'CHANGELOG.md',
    engines: {loom: extension.engines ?? '>=0.1.8 <0.3'},
    ...(runtime ? {} : {capabilities: ['process.execute-local']}),
    ...(extension.requires?.length ? {requires: extension.requires} : {}),
    ...(executables.length ? {executables} : {}),
    contributes: runtime ? {} : {language_servers: servers.map(([serverId, server]) => serverDeclaration(id, serverId, server))},
  };
}

/** Build `id` for every platform (or those in `platforms`). Returns the platforms made. */
export async function build(id, recipe, {dist = DIST, work = WORK, offline = false, root = EXTENSIONS, platforms} = {}) {
  const problems = checkRecipe(id, recipe, {root});
  assert(!problems.length, `${id}:\n  ${problems.join('\n  ')}`);
  const config = loadConfig();
  const pack = recipe.pack, pins = readPins(id, root);
  const all = packPlatforms(pack);
  const chosen = platforms?.length ? platforms : all;
  for (const platform of chosen) assert(all.includes(platform), `${id} is not built for ${platform}`);
  for (const platform of chosen) {
    const directory = stageDirectory(id, platform, dist);
    removeUnder(stageRoot(dist), directory);
    fs.mkdirSync(directory, {recursive: true});
    let provenance;
    if (pack.kind === 'archive') provenance = await assembleArchive(id, pack, pins, platform, directory, {work, offline});
    else if (pack.kind === 'npm') provenance = assembleNpm(id, directory, {work, root});
    else provenance = assembleGo(pack, platform, directory, {work});
    for (const license of pack.licenses ?? []) {
      const pin = pins.licenses[license.name];
      const file = await fetchPinned(pin.url, pin.sha256, path.join(work, 'downloads', pin.sha256), {offline});
      fs.mkdirSync(path.join(directory, 'licenses'), {recursive: true});
      fs.copyFileSync(file, path.join(directory, 'licenses', license.name));
    }
    const executables = pack.kind === 'npm' ? [] : pack.platforms[platform].executables;
    for (const name of executables) {
      const file = path.join(directory, name);
      assert(fs.existsSync(file) && fs.statSync(file).isFile(), `${id} ${platform}: the archive has no ${name}`);
      fs.chmodSync(file, 0o755);
    }
    fs.writeFileSync(path.join(directory, 'PROVENANCE.md'),
      `# ${recipe.extension.name} ${pack.version}\n\nBuilt by loom-extensions.\n\nKind: ${pack.kind}\nPlatform: ${platform}\nLicense: ${recipe.extension.license}\n` +
      `${recipe.extension.requires?.length ? `Requires: ${recipe.extension.requires.join(', ')}\n` : ''}\n${provenance}`);
    const manifest = serverManifest(id, recipe, {repository: config.repository, executables});
    fs.writeFileSync(path.join(directory, 'manifest.json'), `${JSON.stringify(manifest, null, 2)}\n`);
    for (const [file, content] of Object.entries(serverDocuments(id, recipe, platform))) fs.writeFileSync(path.join(directory, file), content);
    const checked = checkExtension(directory, {built: true});
    assert(!checked.problems.length, `${id} (${platform}):\n  ${checked.problems.join('\n  ')}`);
  }
  return chosen;
}
