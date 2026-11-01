// GitHub Releases through the `gh` command line. The command is a parameter so the
// tests can stand in for it. A release is created as a draft, its assets are
// uploaded and checked, and only then is it published; a published release is
// never changed.
import {execFileSync} from 'node:child_process';
import {assert} from './util.mjs';

export const REGISTRY_TAG = 'registry';

export function ghRun(args) {
  return execFileSync('gh', args, {encoding: 'utf8', maxBuffer: 32 * 1024 * 1024});
}

export class GitHubReleases {
  constructor(repository, run = ghRun) {
    assert(/^[\w.-]+\/[\w.-]+$/.test(repository), 'Invalid repository');
    this.repository = repository;
    this.run = run;
  }

  /** Full release inventory for an explicitly requested early-stage reset. */
  list() {
    return JSON.parse(this.run(['api', 'repos/' + this.repository + '/releases', '--paginate', '--slurp'])).flat();
  }

  listTags() {
    return JSON.parse(this.run(['api', 'repos/' + this.repository + '/tags?per_page=100', '--paginate', '--slurp'])).flat().map(tag => tag.name);
  }

  delete(tag) {
    // Drafts need not have a Git tag. A previous attempt may also have removed
    // the release before failing to remove its tag, so delete each independently.
    try {
      this.run(['release', 'delete', tag, '--repo', this.repository, '--yes']);
    } catch (error) {
      const message = String(error.stderr ?? error.message);
      if (!/release not found|HTTP 404\b/i.test(message)) throw error;
    }
    try {
      this.run(['api', '--method', 'DELETE', 'repos/' + this.repository + '/git/refs/tags/' + encodeURIComponent(tag)]);
    } catch (error) {
      const message = String(error.stderr ?? error.message);
      if (!/HTTP 404\b/i.test(message) && !(/HTTP 422\b/i.test(message) && /Reference does not exist/i.test(message))) throw error;
    }
  }

  /** The release for `tag` (drafts included), or null. */
  view(tag) {
    try {
      const data = JSON.parse(this.run(['release', 'view', tag, '--repo', this.repository, '--json', 'tagName,isDraft,assets']));
      return {tag: data.tagName, draft: data.isDraft, assets: data.assets ?? []};
    } catch (error) {
      if (/release not found|HTTP 404|Not Found/i.test(String(error.stderr ?? error.message))) return null;
      throw error;
    }
  }

  createDraft(tag, title, notes, {target} = {}) {
    this.run(['release', 'create', tag, '--repo', this.repository, '--draft', '--latest=false', '--title', title, '--notes', notes, ...(target ? ['--target', target] : [])]);
  }

  /** Create the registry release if it is missing: the one mutable release, holding the index. */
  ensureRegistry() {
    if (this.view(REGISTRY_TAG)) return;
    this.run(['release', 'create', REGISTRY_TAG, '--repo', this.repository, '--latest=false', '--title', 'Extension index', '--notes', 'The signed index of Loom extensions. Replaced on every publication.']);
  }

  upload(tag, files, {clobber = false} = {}) {
    this.run(['release', 'upload', tag, ...files, '--repo', this.repository, ...(clobber ? ['--clobber'] : [])]);
  }

  publish(tag) {
    this.run(['release', 'edit', tag, '--repo', this.repository, '--draft=false']);
  }

  /** Whether the release's assets are exactly `expected` ({name: {sha256, size}}). Returns the differences. */
  differences(release, expected) {
    const found = new Map(release.assets.map(asset => [asset.name, asset]));
    const problems = [];
    for (const [name, want] of Object.entries(expected)) {
      const asset = found.get(name);
      if (!asset) { problems.push(`missing ${name}`); continue; }
      if (asset.size !== want.size) problems.push(`${name}: size ${asset.size}, expected ${want.size}`);
      const digest = asset.digest?.replace(/^sha256:/, '');
      if (digest && digest !== want.sha256) problems.push(`${name}: SHA-256 differs`);
    }
    for (const name of found.keys()) if (!(name in expected)) problems.push(`unexpected ${name}`);
    return problems;
  }
}
