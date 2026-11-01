// Automatic publication is version-driven. Published packages are immutable;
// a release that was uploaded before an interrupted index update is recovered.
import fs from 'node:fs';
import path from 'node:path';
import {execFileSync} from 'node:child_process';
import {setTimeout as pause} from 'node:timers/promises';
import {ROOT, DIST, WORK, assert, fileHash, hash, writeAtomic} from './util.mjs';
import {artifactName} from './platform.mjs';
import {parseRequirement} from './manifest.mjs';
import {satisfies} from './semver.mjs';
import {buildIndex, releaseTag, validateIndex} from './index.mjs';
import {verifyIndex, signIndex, publicKeyOf} from './sign.mjs';
import {GitHubReleases} from './github.mjs';

const meaningful = name => !/\.md$/i.test(name);
const git = (root, args) => execFileSync('git', args, {cwd: root, encoding: 'utf8', maxBuffer: 32 * 1024 * 1024});
export const revision = (root = ROOT) => git(root, ['rev-parse', 'HEAD']).trim();
export const releaseDirectory = (work = WORK) => path.join(work, 'release');

/** Only the extension's own packaging inputs require a version bump. Shared
 * builder/toolchain changes affect future builds; documentation alone is skipped. */
export function fingerprint(root, id, ref) {
  const prefix = `extensions/${id}/`;
  const entries = ref
    ? git(root, ['ls-tree', '-r', '--name-only', '-z', ref, '--', prefix]).split('\0').filter(Boolean)
    // What git tracks or would track: ignored folders (dependencies, build output) are not inputs.
    : git(root, ['ls-files', '-z', '-c', '-o', '--exclude-standard', '--', prefix]).split('\0').filter(file => file && fs.existsSync(path.join(root, file)));
  return hash(JSON.stringify(entries.filter(meaningful).sort().map(file => [file.slice(prefix.length),
    ref ? hash(execFileSync('git', ['show', `${ref}:${file}`], {cwd: root, maxBuffer: 32 * 1024 * 1024})) : fileHash(path.join(root, file))])));
}

async function get(url, fetcher = fetch, headers = {}) {
  const fresh = url + (url.includes('?') ? '&' : '?') + '_check=' + Date.now() + '-' + Math.random().toString(16).slice(2);
  const response = await fetcher(fresh, {headers: {'Cache-Control': 'no-cache', ...headers}, signal: AbortSignal.timeout(120000)});
  assert(response.ok, `${url}: HTTP ${response.status}`);
  return response;
}

/** Verified immutable generations let an interrupted current-pair update recover.
 * Transport failures never become an empty registry or erase its history. */
export async function readRegistry(config, fetcher = fetch) {
  const base = `https://github.com/${config.repository}/releases/download/registry/`;
  const readPair = async (name, signatureName) => {
    const [body, signature] = await Promise.all([get(base + name, fetcher), get(base + signatureName, fetcher)]);
    const bytes = Buffer.from(await body.arrayBuffer());
    assert(bytes.length <= 16 * 1024 * 1024, 'Registry is too large');
    const signatureText = await signature.text();
    const checked = verifyIndex(bytes, signatureText, config.index.keys ?? {});
    assert(checked.ok, `Registry signature: ${checked.reason}`);
    const index = JSON.parse(bytes);
    assert(index.keyId === checked.keyId, 'Registry keyId does not match its signature');
    assert(!validateIndex(index).length, `Invalid registry: ${validateIndex(index).join('; ')}`);
    return {index, bytes, signature: signatureText};
  };
  const [stable, metadata] = await Promise.allSettled([
    readPair('index.json', 'index.json.sig'), readRelease(config.repository, 'registry', fetcher),
  ]);
  if (metadata.status === 'rejected') throw metadata.reason;
  assert(metadata.value, 'The registry release does not exist');
  const names = new Set((metadata.value.assets ?? []).map(asset => asset.name));
  const generations = [...new Set([...names].map(name => /^index-(\d+)\.json(?:\.sig)?$/.exec(name)).filter(Boolean)
    .map(match => Number(match[1])).filter(sequence => Number.isSafeInteger(sequence) && sequence > 0))].sort((a, b) => b - a);
  let selected = stable.status === 'fulfilled' ? stable.value : null;
  for (const sequence of generations) {
    if (selected && sequence < selected.index.sequence) break;
    if (!names.has(`index-${sequence}.json.sig`)) continue;
    let generation;
    try { generation = await readPair(`index-${sequence}.json`, `index-${sequence}.json.sig`); }
    catch { continue; }
    assert(generation.index.sequence === sequence, 'Registry generation filename has the wrong sequence');
    if (selected?.index.sequence === sequence) assert(selected.bytes.equals(generation.bytes), 'Conflicting signed registry generations');
    if (!selected || sequence > selected.index.sequence) selected = generation;
  }
  if (!selected) throw stable.status === 'rejected' ? stable.reason : new Error('No verified registry generation');
  const recovered = stable.status !== 'fulfilled' || !stable.value.bytes.equals(selected.bytes) || stable.value.signature !== selected.signature;
  return {...selected, recovered, maxSequence: Math.max(selected.index.sequence, ...generations)};
}

export async function readRelease(repository, tag, fetcher = fetch, {id} = {}) {
  const token = process.env.GH_TOKEN || process.env.GITHUB_TOKEN;
  const headers = {Accept: 'application/vnd.github+json', 'Cache-Control': 'no-cache', ...(token ? {Authorization: 'Bearer ' + token} : {})};
  const base = 'https://api.github.com/repos/' + repository + '/releases';
  const read = url => fetcher(url + (url.includes('?') ? '&' : '?') + '_check=' + Date.now() + '-' + Math.random().toString(16).slice(2),
    {headers, signal: AbortSignal.timeout(120000)});
  if (id !== undefined) {
    assert(Number.isSafeInteger(id) && id > 0, 'Invalid release ID');
    const response = await read(base + '/' + id);
    if (response.status === 404) return null;
    assert(response.ok, 'Reading release ' + id + ': HTTP ' + response.status);
    const release = await response.json();
    assert(release.id === id && release.tag_name === tag, 'Release ID or tag does not match the expected release');
    return release;
  }
  const response = await read(base + '/tags/' + encodeURIComponent(tag));
  if (response.status !== 404) {
    assert(response.ok, 'Reading ' + tag + ': HTTP ' + response.status);
    return response.json();
  }
  // The tag endpoint is for published releases. Authenticated listings also
  // include drafts, whose tag may not exist until publication. Never cache the
  // pre-create 404 or a pre-upload asset listing during readback verification.
  for (let page = 1; ; page++) {
    const listing = await read(base + '?per_page=100&page=' + page);
    assert(listing.ok, 'Reading release inventory for ' + tag + ': HTTP ' + listing.status);
    const releases = await listing.json();
    assert(Array.isArray(releases), 'Invalid GitHub release inventory');
    const found = releases.find(release => release.tag_name === tag);
    if (found) return found;
    if (releases.length < 100) return null;
  }
}

export function checkAssets(release, records) {
  assert(release, 'Release disappeared');
  const assets = new Map(release.assets.map(a => [a.name, a]));
  assert(assets.size === records.length && release.assets.length === records.length, 'Release platform assets are incomplete or unexpected');
  for (const record of records) {
    const asset = assets.get(record.file);
    assert(asset && asset.size === record.size && asset.digest === `sha256:${record.sha256}`, `${record.file}: remote size or SHA-256 does not match`);
  }
}

/** Read a known release after writes without depending on tag/list visibility.
 * Retry only verification of the just-written resource; do not repeat uploads. */
export async function verifyRelease(repository, tag, id, records, {lookup = readRelease, published = false, wait = pause} = {}) {
  assert(Number.isSafeInteger(id) && id > 0, 'Publication requires a stable release ID');
  for (let attempt = 0; ; attempt++) {
    const remote = await lookup(repository, tag, undefined, {id});
    try {
      assert(remote, 'Release metadata is not visible yet');
      assert(remote.id === id && remote.tag_name === tag, 'Release ID or tag does not match the expected release');
      checkAssets(remote, records);
      if (published) assert(!remote.draft, 'Release is still a draft');
      return remote;
    } catch (error) {
      if (attempt === 2) throw new Error(tag + ' (release ' + id + '): ' + error.message);
      await wait([500, 1500][attempt]);
    }
  }
}

export function checkRecords(item, records, directory) {
  assert(records.length === item.platforms.length, `${item.id}: incomplete platforms`);
  assert(new Set(records.map(r => r.platform)).size === records.length, `${item.id}: duplicate platforms`);
  for (const record of records) {
    assert(record.id === item.id && record.version === item.version && item.platforms.includes(record.platform), `${item.id}: unexpected package identity or platform`);
    assert(record.file === artifactName(item.id, item.version, record.platform), `${item.id}: unexpected package filename`);
    const file = path.join(directory, record.file);
    assert(fileHash(file) === record.sha256 && fs.statSync(file).size === record.size, `${record.file}: local size or SHA-256 does not match`);
  }
}

/** Only versions whose required extensions exist can be made discoverable. */
export function readyToIndex(prepared, previous) {
  const ready = [];
  let pending = [...prepared];
  while (pending.length) {
    const next = [];
    for (const item of pending) {
      const available = (item.description.manifest.requires ?? []).every(text => {
        const requirement = parseRequirement(text);
        assert(requirement, `${item.id}: invalid requirement ${text}`);
        const candidate = prepared.find(other => other.id === requirement.id && satisfies(other.version, requirement.range));
        if (candidate) return ready.includes(candidate);
        return previous.extensions.some(e => e.id === requirement.id && e.versions.some(v => satisfies(v.version, requirement.range)));
      });
      if (available) ready.push(item); else next.push(item);
    }
    if (next.length === pending.length) break;
    pending = next;
  }
  return ready;
}

/** Publish complete extension releases before making them visible in the index. */
export function checkSigningKey(config, privateKey) {
  assert(privateKey, 'LOOM_INDEX_KEY is required; automatic publication never produces an unsigned index');
  assert(config.index.keys?.[config.index['key-id']] === publicKeyOf(privateKey), 'Signing key does not match the trusted public key');
}

export async function finishRelease(plan, outcomes, {config, privateKey, root = ROOT, dist = DIST, work = WORK,
  releases = new GitHubReleases(config.repository), registry = readRegistry, lookup = readRelease,
  now = new Date(), sourceRevision = plan.revision, wait = pause} = {}) {
  checkSigningKey(config, privateKey);
  assert(sourceRevision === plan.revision, 'Publisher is not on the planned commit');
  const expected = new Map(plan.pending.map(item => [item.id, item]));
  const initial = await registry(config);
  assert(initial.index.sequence >= plan.sequence, 'Registry sequence went backwards');
  const published = [], failures = [], seen = new Set(), checked = [];
  for (const item of outcomes) {
    const planned = expected.get(item.id);
    assert(planned && !seen.has(item.id), 'Unexpected or duplicate release result');
    seen.add(item.id);
    if (item.error) { failures.push({id: item.id, error: item.error}); continue; }
    assert(fingerprint(root, item.id) === planned.fingerprint, `${item.id}: inputs changed after planning`);
    assert(item.version === planned.version && item.source.fingerprint === planned.fingerprint, 'Build provenance differs from the plan');
    checkRecords(planned, item.records, path.join(dist, 'packages'));
    assert(item.description.manifest.id === item.id && item.description.manifest.version === planned.version, 'Release description differs from the plan');
    checked.push(item);
  }
  for (const id of expected.keys()) if (!seen.has(id)) failures.push({id, error: 'No build result'});
  const eligible = readyToIndex(checked, initial.index);
  for (const item of checked) if (!eligible.includes(item)) failures.push({id: item.id, error: 'Required extension is missing or dependencies cycle'});
  for (const item of eligible) {
    if (!readyToIndex([...published, item], initial.index).includes(item)) {
      failures.push({id: item.id, error: 'Required extension failed publication'});
      continue;
    }
    try {
      const tag = releaseTag(item.id, item.version);
      let remote = await lookup(config.repository, tag);
      if (remote && !remote.draft) checkAssets(remote, item.records);
      else {
        if (!remote) remote = releases.createDraft(tag, `${item.id} ${item.version}`, `Release of ${item.id} ${item.version}.`, {target: plan.revision});
        assert(Number.isSafeInteger(remote.id) && remote.id > 0, 'Draft has no stable release ID');
        const id = remote.id;
        releases.upload(tag, item.records.map(r => path.join(dist, 'packages', r.file)), {clobber: true});
        await verifyRelease(config.repository, tag, id, item.records, {lookup, wait});
        releases.publish(tag, {id});
        await verifyRelease(config.repository, tag, id, item.records, {lookup, wait, published: true});
      }
      published.push(item);
    } catch (error) { failures.push({id: item.id, error: error.message}); }
  }
  // Re-read inside the workflow's exclusive publication slot, never merge an old snapshot.
  const fresh = await registry(config);
  assert(fresh.index.sequence >= plan.sequence, 'Registry sequence went backwards');
  const ready = readyToIndex(published, fresh.index);
  for (const item of published) if (!ready.includes(item)) failures.push({id: item.id, error: 'Required extension is not available; release kept for recovery'});
  const additions = ready.filter(item => {
    const old = fresh.index.extensions.find(e => e.id === item.id)?.versions.find(v => v.version === item.version);
    if (!old) return true;
    assert(old.artifacts.length === item.records.length && item.records.every(record => old.artifacts.some(a =>
      a.platform === record.platform && a.sha256 === record.sha256 && a.size === record.size)), `${item.id}: indexed bytes differ from the release`);
    return false;
  });
  let result = fresh;
  if (additions.length) {
    const index = buildIndex({packages: additions.flatMap(item => item.records), repository: config.repository,
      keyId: config.index['key-id'], validityDays: config.index['validity-days'], previous: fresh.index, now, dist,
      descriptions: Object.fromEntries(additions.map(item => [item.id, item.description])),
      sources: Object.fromEntries(additions.map(item => [item.id, {...item.source, revision: plan.revision}]))});
    index.sequence = (fresh.maxSequence ?? fresh.index.sequence) + 1;
    assert(!validateIndex(index).length, `Invalid generated index: ${validateIndex(index).join('; ')}`);
    const bytes = Buffer.from(JSON.stringify(index, null, 2) + '\n');
    const signature = signIndex(bytes, privateKey, config.index['key-id']);
    const indexPath = path.join(dist, 'index/index.json'), signaturePath = path.join(dist, 'index/index.json.sig');
    writeAtomic(indexPath, bytes); writeAtomic(signaturePath, signature);
    const generationPath = path.join(dist, 'index', `index-${index.sequence}.json`);
    const generationSignature = generationPath + '.sig';
    writeAtomic(generationPath, bytes); writeAtomic(generationSignature, signature);
    // Keep an immutable complete generation before replacing either current file.
    releases.upload('registry', [generationPath, generationSignature]);
    const stagedGeneration = await registry(config);
    assert(stagedGeneration.bytes.equals(bytes) && stagedGeneration.signature === signature, 'Immutable registry generation did not verify');
    releases.upload('registry', [indexPath, signaturePath], {clobber: true});
    result = await registry(config);
    assert(result.bytes.equals(bytes) && result.signature === signature, 'Published index/signature did not match the verified output');
  } else {
    writeAtomic(path.join(dist, 'index/index.json'), fresh.bytes);
    writeAtomic(path.join(dist, 'index/index.json.sig'), fresh.signature);
    if (fresh.recovered) {
      releases.upload('registry', [path.join(dist, 'index/index.json'), path.join(dist, 'index/index.json.sig')], {clobber: true});
      result = await registry(config);
      assert(!result.recovered && result.bytes.equals(fresh.bytes) && result.signature === fresh.signature, 'Registry pair repair did not complete');
    }
  }
  const report = {schema: 1, revision: plan.revision, sequence: result.index.sequence, indexed: ready.map(item => item.id), failures};
  writeAtomic(path.join(releaseDirectory(work), 'report.json'), JSON.stringify(report, null, 2) + '\n');
  return report;
}
