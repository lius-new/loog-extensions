// Intake: what is published is a package, not the way it was made. Each `.loomext`
// handed over is unpacked and must be a package Loom can import; how its project
// was laid out or built is not looked at.
import fs from 'node:fs';
import path from 'node:path';
import {DIST, ROOT, WORK, assert, fileHash, removeUnder} from './util.mjs';
import {extractPackage} from './archive.mjs';
import {checkExtension} from './manifest.mjs';
import {artifactName, validPlatform} from './platform.mjs';
import {describe, releaseTag} from './index.mjs';
import {clearStage, stageDirectory} from './stage.mjs';
import {runValidator} from './validator.mjs';
import {compare} from './semver.mjs';
import {fetchPinned} from './fetch.mjs';
import {fingerprint, readRelease} from './release.mjs';

const SHA = /^[a-f0-9]{40}$/;

/** `.loomext` files named on the command line, or found in the folders named. */
export function packageFiles(paths) {
  return paths.flatMap(item => (fs.statSync(item).isDirectory()
    ? fs.readdirSync(item).filter(name => name.endsWith('.loomext')).sort().map(name => path.resolve(item, name))
    : [path.resolve(item)]));
}

/** The platform in `<id>-<version>-<platform>.loomext`. */
export function platformOf(file, manifest) {
  const name = path.basename(file), prefix = `${manifest.id}-${manifest.version}-`;
  assert(name.startsWith(prefix) && name.endsWith('.loomext'), `${name}: the file is named ${prefix}<platform>.loomext`);
  const platform = name.slice(prefix.length, -'.loomext'.length);
  assert(validPlatform(platform), `${name}: ${platform} is not a platform`);
  assert(artifactName(manifest.id, manifest.version, platform) === name, `${name}: unexpected file name`);
  return platform;
}

async function unpack(files, scratch, work) {
  const groups = new Map();
  for (const [number, file] of files.entries()) {
    const destination = path.join(scratch, String(number));
    removeUnder(work, destination);
    const directory = await extractPackage(file, destination);
    const {manifest, problems} = checkExtension(directory, {built: true});
    assert(!problems.length, `${path.basename(file)}:\n  ${problems.join('\n  ')}`);
    assert(path.basename(directory) === manifest.id, `${path.basename(file)}: the package root is not named ${manifest.id}`);
    const platform = platformOf(file, manifest);
    const group = groups.get(manifest.id) ?? {id: manifest.id, items: []};
    group.items.push({file, directory, manifest, platform});
    groups.set(manifest.id, group);
  }
  return [...groups.values()];
}

/**
 * Check every package and prepare each extension's publication. One result per
 * extension: `{id, version, records, description, source, recovered}`, `{id, skipped}`
 * for a version that is already published as it is, or `{id, error}`.
 * `index` (the current signed index) and `repository` enable the checks against what
 * is already published; without them only the packages themselves are checked.
 */
export async function intake(files, {root = ROOT, dist = DIST, work = WORK, validator, index = null, repository = null,
  lookup = readRelease, validate = runValidator, download = fetchPinned} = {}) {
  assert(files.length, 'No packages were given');
  const groups = await unpack(files, path.join(work, 'intake'), work);
  const results = [];
  for (const group of groups) {
    const {id} = group;
    try {
      const [{manifest: {version}}] = group.items;
      assert(group.items.every(item => item.manifest.version === version), `${id}: the packages are of different versions`);
      assert(new Set(group.items.map(item => item.platform)).size === group.items.length, `${id}: two packages for one platform`);
      // What the extension's sources were; only publishing needs it, and it needs a git checkout.
      let print = null;
      try { print = fingerprint(root, id); } catch (error) { if (index || repository) throw error; }
      const published = index?.extensions.find(e => e.id === id)?.versions.find(v => v.version === version);
      if (published) {
        assert(!(published.source?.fingerprint && published.source.fingerprint !== print), `${id} ${version}: the extension changed since it was published; bump its version`);
        results.push({id, skipped: `${version} is already published`});
        continue;
      }
      const versions = index?.extensions.find(e => e.id === id)?.versions ?? [];
      assert(versions.every(v => compare(version, v.version) > 0), `${id}: a new version must be newer than the published versions`);
      for (const item of group.items) if (validator && item.manifest.contributes?.languages?.length) {
        // Loom's own loaders must accept it; a sample is optional and not required.
        validate(validator, item.directory, null);
      }
      const remote = repository ? await lookup(repository, releaseTag(id, version)) : null;
      const records = [];
      for (const item of group.items) {
        const output = path.join(dist, 'packages', path.basename(item.file));
        fs.mkdirSync(path.dirname(output), {recursive: true});
        fs.copyFileSync(item.file, output);
        records.push({id, version, platform: item.platform, file: path.basename(output), sha256: fileHash(output), size: fs.statSync(output).size});
      }
      const staged = new Map(group.items.map(item => [item.platform, item.directory]));
      if (remote && !remote.draft) {
        // Published before the index was updated (an interrupted delivery). A published
        // version never changes: when the sources are those it was released from, what
        // went out is what is indexed, whatever this run built.
        const tag = releaseTag(id, version);
        if (SHA.test(remote.target_commitish)) assert(fingerprint(root, id, remote.target_commitish) === print, `${tag}: the extension differs from its released commit; bump its version`);
        assert(remote.assets.length === records.length && remote.assets.every(asset => records.some(record => record.file === asset.name)), `${tag}: the release has other platforms than the packages`);
        for (const record of records) {
          const asset = remote.assets.find(candidate => candidate.name === record.file);
          assert(/^sha256:[a-f0-9]{64}$/.test(asset.digest ?? ''), `${record.file}: GitHub supplied no SHA-256`);
          if (asset.digest === `sha256:${record.sha256}` && asset.size === record.size) continue;
          const file = path.join(dist, 'packages', record.file);
          await download(asset.browser_download_url, asset.digest.slice(7), file);
          Object.assign(record, {sha256: asset.digest.slice(7), size: asset.size});
          staged.set(record.platform, await extractPackage(file, path.join(work, 'intake', 'published', record.platform)));
        }
      }
      clearStage(id, dist);
      for (const [platform, directory] of staged) {
        const stage = stageDirectory(id, platform, dist);
        fs.mkdirSync(path.dirname(stage), {recursive: true});
        fs.cpSync(directory, stage, {recursive: true});
      }
      results.push({id, version, records, description: describe(id, dist), source: {fingerprint: print},
        recovered: Boolean(remote && !remote.draft)});
    } catch (error) {
      results.push({id, error: error.message});
    }
  }
  removeUnder(work, path.join(work, 'intake'));
  return results;
}
