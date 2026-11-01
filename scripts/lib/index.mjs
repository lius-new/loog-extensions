// The registry index: every published extension with its versions and, for each
// version, the package per platform. Loom fetches this one file (and its signature)
// and decides from it what to offer and which package to download.
import fs from 'node:fs';
import path from 'node:path';
import {DIST, HASH, assert, readJson} from './util.mjs';
import {checkExtension, validId} from './manifest.mjs';
import {compare, validRange, validVersion} from './semver.mjs';
import {stageDirectory, stagedPlatforms} from './stage.mjs';
import {validPlatform} from './platform.mjs';

export const SCHEMA = 1;

/** Where the package `file` of release `tag` is downloaded from. */
export const downloadUrl = (repository, tag, file) => `https://github.com/${repository}/releases/download/${encodeURIComponent(tag)}/${encodeURIComponent(file)}`;
export const releaseTag = (id, version) => `${id}-${version}`;

/** Largest icon, README and changelog the index carries; the index as a whole is limited too. */
export const MAX_ICON = 32 * 1024;
export const MAX_DOCUMENT = 64 * 1024;

/**
 * What the list shows before anything is installed: the icon (SVG as text, PNG as
 * base64), the README and the changelog, read from the staged package.
 */
function preview(id, directory, manifest) {
  const read = (relative, limit, what) => {
    const bytes = fs.readFileSync(path.join(directory, ...relative.split('/')));
    assert(bytes.length <= limit, `${id}: the ${what} is ${bytes.length} bytes; the index carries at most ${limit}`);
    return bytes;
  };
  const result = {};
  if (manifest.icon) {
    const svg = manifest.icon.toLowerCase().endsWith('.svg');
    const bytes = read(manifest.icon, MAX_ICON, 'icon');
    result.icon = {format: svg ? 'svg' : 'png', data: svg ? bytes.toString('utf8') : bytes.toString('base64')};
  }
  if (manifest.readme) result.readme = read(manifest.readme, MAX_DOCUMENT, 'README').toString('utf8');
  if (manifest.changelog) result.changelog = read(manifest.changelog, MAX_DOCUMENT, 'changelog').toString('utf8');
  return result;
}

/** What the index says about an extension, taken from a staged copy of it. */
export function describe(id, dist = DIST) {
  const platform = stagedPlatforms(id, dist)[0];
  assert(platform, `${id}: nothing is staged; run build first`);
  const directory = stageDirectory(id, platform, dist);
  const {manifest, problems} = checkExtension(directory, {built: true});
  assert(!problems.length, `${id}:\n  ${problems.join('\n  ')}`);
  const entry = {
    id, name: manifest.name, publisher: manifest.publisher, description: manifest.description,
    categories: manifest.categories, keywords: manifest.keywords ?? [], license: manifest.license,
  };
  const languages = [];
  for (const resource of manifest.contributes?.languages ?? []) {
    const pack = path.join(directory, ...resource.path.split('/'));
    for (const grammar of readJson(pack).grammars ?? []) languages.push({name: grammar.name, fileTypes: grammar['file-types'] ?? []});
  }
  if (languages.length) entry.languages = languages;
  Object.assign(entry, preview(id, directory, manifest));
  const servers = (manifest.contributes?.language_servers ?? []).flatMap(server => server.languages);
  if (servers.length) entry.serves = [...new Set(servers)].sort();
  return {entry, manifest};
}

/** The index for the packages in `packages`, extending `previous` (the last published one). */
export function buildIndex({packages, repository, keyId, validityDays, now = new Date(), previous = null, dist = DIST, descriptions = {}, sources = {}}) {
  const byExtension = new Map();
  for (const record of packages) {
    if (!byExtension.has(record.id)) byExtension.set(record.id, []);
    byExtension.get(record.id).push(record);
  }
  const extensions = new Map((previous?.extensions ?? []).map(extension => [extension.id, structuredClone(extension)]));
  for (const [id, records] of byExtension) {
    const { entry, manifest } = descriptions[id] ?? describe(id, dist);
    const published = extensions.get(id);
    const versions = new Map((published?.versions ?? []).map(version => [version.version, version]));
    const version = records[0].version;
    assert(records.every(record => record.version === version), `${id}: staged packages are of different versions`);
    const artifacts = records.map(record => ({platform: record.platform, url: downloadUrl(repository, releaseTag(id, version), record.file), sha256: record.sha256, size: record.size}))
      .sort((a, b) => (a.platform < b.platform ? -1 : 1));
    const old = versions.get(version);
    if (old) {
      // A published version never changes.
      assert(JSON.stringify(old.artifacts) === JSON.stringify(artifacts), `${id} ${version} is already published with different contents; release a new version`);
    }
    versions.set(version, {
      version, engines: manifest.engines, capabilities: manifest.capabilities ?? [], requires: manifest.requires ?? [],
      recommends: manifest.recommends ?? [], artifacts,
      ...((old?.source ?? sources[id]) ? {source: old?.source ?? sources[id]} : {}),
    });
    extensions.set(id, {...entry, versions: [...versions.values()].sort((a, b) => compare(b.version, a.version))});
  }
  const generated = now.toISOString();
  const expires = new Date(now.getTime() + validityDays * 86400000).toISOString();
  return {
    schema: SCHEMA, generated, expires, sequence: (previous?.sequence ?? 0) + 1, keyId,
    extensions: [...extensions.values()].sort((a, b) => (a.id < b.id ? -1 : 1)),
  };
}

/** Problems with an index; empty when it is well formed. */
export function validateIndex(index) {
  const problems = [];
  const bad = message => problems.push(message);
  if (index?.schema !== SCHEMA) bad(`schema must be ${SCHEMA}`);
  if (!Number.isSafeInteger(index?.sequence) || index.sequence < 1) bad('sequence must be a positive integer');
  if (Number.isNaN(Date.parse(index?.generated)) || Number.isNaN(Date.parse(index?.expires))) bad('generated and expires must be dates');
  else if (Date.parse(index.expires) <= Date.parse(index.generated)) bad('expires must be after generated');
  if (!/^[\w.-]+$/.test(index?.keyId ?? '')) bad('keyId is required');
  const seen = new Set();
  for (const extension of index?.extensions ?? []) {
    const here = message => bad(`${extension.id}: ${message}`);
    if (!validId(extension.id)) { bad(`invalid extension id ${JSON.stringify(extension.id)}`); continue; }
    if (seen.has(extension.id)) here('listed twice');
    seen.add(extension.id);
    if (extension.icon !== undefined && (!['svg', 'png'].includes(extension.icon?.format) || typeof extension.icon.data !== 'string' || extension.icon.data.length > MAX_ICON * 2)) here('invalid icon');
    for (const field of ['readme', 'changelog']) {
      if (extension[field] !== undefined && (typeof extension[field] !== 'string' || Buffer.byteLength(extension[field]) > MAX_DOCUMENT)) here(`invalid ${field}`);
    }
    if (!extension.versions?.length) here('has no versions');
    const versions = new Set();
    for (const version of extension.versions ?? []) {
      if (!validVersion(version.version)) here(`invalid version ${version.version}`);
      if (versions.has(version.version)) here(`version ${version.version} listed twice`);
      versions.add(version.version);
      if (version.engines?.loom !== undefined && !validRange(version.engines.loom)) here(`${version.version}: invalid engines.loom`);
      if (!version.artifacts?.length) here(`${version.version}: no packages`);
      const platforms = new Set();
      for (const artifact of version.artifacts ?? []) {
        if (!validPlatform(artifact.platform)) here(`${version.version}: invalid platform ${artifact.platform}`);
        if (platforms.has(artifact.platform)) here(`${version.version}: platform ${artifact.platform} listed twice`);
        platforms.add(artifact.platform);
        if (!HASH.test(artifact.sha256 ?? '') || !Number.isSafeInteger(artifact.size) || artifact.size <= 0) here(`${version.version}: invalid hash or size for ${artifact.platform}`);
        if (!String(artifact.url ?? '').startsWith('https://')) here(`${version.version}: the URL for ${artifact.platform} must be https`);
      }
      if (platforms.has('any') && platforms.size > 1) here(`${version.version}: a version is either for any platform or for named ones`);
    }
    const order = (extension.versions ?? []).map(version => version.version);
    if (order.some((version, i) => i && validVersion(version) && validVersion(order[i - 1]) && compare(order[i - 1], version) <= 0)) here('versions must be newest first');
  }
  return problems;
}

export const indexFile = (dist = DIST) => path.join(dist, 'index', 'index.json');
export const signatureFile = (dist = DIST) => path.join(dist, 'index', 'index.json.sig');
export const readIndex = file => (fs.existsSync(file) ? JSON.parse(fs.readFileSync(file, 'utf8')) : null);
