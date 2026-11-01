// publish [id…] [--yes]: upload the packages as GitHub releases, then the index.
// Without --yes it only says what it would do. A published release is never
// changed: if one exists with other contents, publishing stops.
import fs from 'node:fs';
import path from 'node:path';
import {DIST, assert} from '../lib/util.mjs';
import {loadConfig} from '../lib/config.mjs';
import {GitHubReleases, REGISTRY_TAG} from '../lib/github.mjs';
import {indexFile, releaseTag, signatureFile} from '../lib/index.mjs';
import {readPackages} from './pack.mjs';
import {parseArgs} from '../lib/cli.mjs';

export const usage = 'publish [id…] [--yes] [--no-index]    Upload packages and the index to GitHub Releases (a dry run without --yes)';

/** What publishing the given package records means for each release. */
export function planReleases(records, releases) {
  const byTag = new Map();
  for (const record of records) {
    const tag = releaseTag(record.id, record.version);
    if (!byTag.has(tag)) byTag.set(tag, {tag, id: record.id, version: record.version, assets: {}, files: []});
    const entry = byTag.get(tag);
    entry.assets[record.file] = {sha256: record.sha256, size: record.size};
    entry.files.push(record.file);
  }
  return [...byTag.values()].map(entry => {
    const release = releases.view(entry.tag);
    if (!release) return {...entry, action: 'create', upload: entry.files};
    const differences = releases.differences(release, entry.assets);
    if (!release.draft) {
      assert(!differences.length, `${entry.tag} is already published with different contents (${differences.join('; ')}); release a new version`);
      return {...entry, action: 'skip', upload: []};
    }
    const present = new Set(release.assets.map(asset => asset.name));
    return {...entry, action: 'resume', upload: entry.files.filter(file => !present.has(file))};
  });
}

export async function run(argv, {releases, dist = DIST, repository} = {}) {
  const {options, ids} = parseArgs(argv, {yes: 'boolean', 'no-index': 'boolean'});
  const listing = readPackages(dist);
  const records = listing.packages.filter(record => !ids.length || ids.includes(record.id));
  assert(records.length, 'No packages to publish; run build and pack first');
  const github = releases ?? new GitHubReleases(repository ?? loadConfig().repository);
  const plan = planReleases(records, github);
  for (const step of plan) console.log(`${options.yes ? '' : '[dry run] '}${step.tag}: ${step.action}${step.upload.length ? ` (${step.upload.length} file(s))` : ''}`);
  const withIndex = !options['no-index'] && fs.existsSync(indexFile(dist));
  if (withIndex) console.log(`${options.yes ? '' : '[dry run] '}${REGISTRY_TAG}: replace index.json${fs.existsSync(signatureFile(dist)) ? ' and index.json.sig' : ' (unsigned)'}`);
  if (!options.yes) { console.log('Nothing was changed. Pass --yes to publish.'); return; }
  for (const step of plan) {
    if (step.action === 'skip') continue;
    if (step.action === 'create') github.createDraft(step.tag, `${step.id} ${step.version}`, `Release of ${step.id} ${step.version}.`);
    if (step.upload.length) github.upload(step.tag, step.upload.map(file => path.join(dist, 'packages', file)));
    // The release is published only once its assets are exactly what was packed.
    const after = github.view(step.tag);
    const differences = github.differences(after, step.assets);
    assert(!differences.length, `${step.tag}: the uploaded assets do not match (${differences.join('; ')}); it is left as a draft`);
    github.publish(step.tag);
    console.log(`Published ${step.tag}`);
  }
  if (withIndex) {
    github.ensureRegistry();
    const files = [indexFile(dist), ...(fs.existsSync(signatureFile(dist)) ? [signatureFile(dist)] : [])];
    github.upload(REGISTRY_TAG, files, {clobber: true});
    console.log('Published the index');
  }
}
