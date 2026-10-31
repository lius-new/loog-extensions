// Resolves the pins a server or runtime recipe needs: the SHA-256 and size of every
// upstream archive and license file, and the lockfile of an npm pack. Run it after
// changing a version in a recipe, then review and commit pins.toml and npm/. Nothing
// is fetched at build time that was not pinned here.
import fs from 'node:fs';
import path from 'node:path';
import {createHash} from 'node:crypto';
import {HASH, EXTENSIONS, assert, run, toToml, writeAtomic} from './util.mjs';
import {assetUrl, pinsFile, readPins} from './build-server.mjs';

const headers = () => ({'User-Agent': 'loom-extensions', ...(process.env.GITHUB_TOKEN ? {Authorization: `Bearer ${process.env.GITHUB_TOKEN}`} : {})});

/** The SHA-256 and size of what `url` serves. */
export async function digestOf(url, fetcher = fetch) {
  const response = await fetcher(url, {redirect: 'follow', headers: headers(), signal: AbortSignal.timeout(600000)});
  assert(response.ok && response.body, `Download failed (${response.status}): ${url}`);
  const hasher = createHash('sha256');
  let size = 0;
  for await (const chunk of response.body) { hasher.update(chunk); size += chunk.length; }
  return {sha256: hasher.digest('hex'), size};
}

async function githubAssets(repository, tag, fetcher) {
  const response = await fetcher(`https://api.github.com/repos/${repository}/releases/tags/${encodeURIComponent(tag)}`, {headers: headers()});
  assert(response.ok, `GitHub release not found (${response.status}): ${repository}@${tag}`);
  return new Map((await response.json()).assets.map(asset => [asset.name, asset]));
}

async function nodeChecksums(version, fetcher) {
  const response = await fetcher(`https://nodejs.org/dist/v${version}/SHASUMS256.txt`);
  assert(response.ok, `Node release not found: ${version}`);
  return new Map((await response.text()).trim().split(/\r?\n/).map(line => { const [sum, name] = line.trim().split(/\s+/); return [name, sum]; }));
}

/** The pin for one archive: from the publisher's own checksum when it gives one, else by hashing the download. */
async function pinArchive(pack, platform, caches, fetcher) {
  const url = assetUrl(pack, platform), spec = pack.platforms[platform];
  if (pack.upstream.source === 'nodejs') {
    caches.node ??= await nodeChecksums(pack.version, fetcher);
    const sum = caches.node.get(spec.asset);
    assert(HASH.test(sum || ''), `No published checksum for ${spec.asset}`);
    const head = await fetcher(url, {method: 'HEAD'});
    return {url, sha256: sum, size: Number(head.headers.get('content-length'))};
  }
  caches.github ??= await githubAssets(pack.upstream.repository, pack.upstream.tag, fetcher);
  const asset = caches.github.get(spec.asset);
  assert(asset, `The release has no asset ${spec.asset}`);
  const digest = asset.digest?.replace(/^sha256:/, '');
  if (HASH.test(digest || '')) return {url, sha256: digest, size: asset.size};
  const measured = await digestOf(url, fetcher);
  assert(measured.size === asset.size, `Downloaded size differs from the release: ${spec.asset}`);
  return {url, ...measured};
}

/** The pins for `recipe`, keeping the old ones unless `refresh` asks to look again. */
export async function pinPack(id, recipe, existing, {refresh = false, fetcher = fetch, log = console.log} = {}) {
  const pack = recipe.pack, pins = {schema: 1};
  if (pack.kind === 'npm') return pins;
  const caches = {};
  if (pack.kind === 'archive') {
    pins.platforms = {};
    for (const platform of Object.keys(pack.platforms)) {
      const url = assetUrl(pack, platform), kept = existing.platforms?.[platform];
      if (kept && kept.url === url && !refresh) { pins.platforms[platform] = kept; continue; }
      log(`pinning ${id} ${platform}`);
      pins.platforms[platform] = await pinArchive(pack, platform, caches, fetcher);
    }
  }
  if (pack.licenses?.length) {
    pins.licenses = {};
    for (const license of pack.licenses) {
      const kept = existing.licenses?.[license.name];
      if (kept && kept.url === license.url && !refresh) { pins.licenses[license.name] = kept; continue; }
      log(`pinning ${id} ${license.name}`);
      pins.licenses[license.name] = {url: license.url, ...(await digestOf(license.url, fetcher))};
    }
  }
  return pins;
}

/** Writes package.json and the lockfile of an npm pack; the lockfile pins every dependency by integrity. */
export function pinNpm(id, recipe, root = EXTENSIONS) {
  const directory = path.join(root, id, 'npm');
  fs.mkdirSync(directory, {recursive: true});
  const dependencies = Object.fromEntries(recipe.pack.packages.map(spec => { const at = spec.lastIndexOf('@'); return [spec.slice(0, at), spec.slice(at + 1)]; }));
  writeAtomic(path.join(directory, 'package.json'), `${JSON.stringify({name: `loom-ext-${id}`, version: '0.0.0', private: true, dependencies}, null, 2)}\n`);
  fs.rmSync(path.join(directory, 'package-lock.json'), {force: true});
  run(process.platform === 'win32' ? 'npm.cmd' : 'npm', ['install', '--package-lock-only', '--ignore-scripts', '--no-audit', '--no-fund'],
    {cwd: directory, shell: process.platform === 'win32', env: {...process.env, npm_config_update_notifier: 'false'}});
}

export function writePins(id, pins, root = EXTENSIONS) {
  if (Object.keys(pins).length <= 1) return;
  writeAtomic(pinsFile(id, root), `# Written by loom-ext pin. Do not edit by hand.\n${toToml(pins)}`);
}

export { readPins };
