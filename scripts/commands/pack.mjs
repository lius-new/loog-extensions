// pack [id…]: turn what `build` staged into .loomext packages, one per platform.
import fs from 'node:fs';
import path from 'node:path';
import {DIST, assert, readJson, selectExtensions, writeAtomic} from '../lib/util.mjs';
import {createPackage} from '../lib/archive.mjs';
import {artifactName} from '../lib/platform.mjs';
import {stageDirectory, stagedPlatforms} from '../lib/stage.mjs';
import {checkExtension} from '../lib/manifest.mjs';
import {parseArgs} from '../lib/cli.mjs';

export const usage = 'pack [id…]    Pack dist/stage into dist/packages/*.loomext and write packages.json';

export const packagesFile = (dist = DIST) => path.join(dist, 'packages', 'packages.json');
export const readPackages = (dist = DIST) => (fs.existsSync(packagesFile(dist)) ? readJson(packagesFile(dist)) : {schema: 1, packages: []});

/** Pack extension `id` for every platform it was staged for. Returns the records. */
export async function packExtension(id, dist = DIST) {
  const platforms = stagedPlatforms(id, dist);
  assert(platforms.length, `${id}: nothing is staged; run build first`);
  const records = [];
  for (const platform of platforms) {
    const directory = stageDirectory(id, platform, dist);
    const {manifest, problems} = checkExtension(directory, {built: true});
    assert(!problems.length, `${id} (${platform}):\n  ${problems.join('\n  ')}`);
    const output = path.join(dist, 'packages', artifactName(id, manifest.version, platform));
    const packed = await createPackage(directory, output, id);
    records.push({id, version: manifest.version, platform, file: path.basename(output), sha256: packed.sha256, size: packed.size});
  }
  return records;
}

export async function run(argv) {
  const {ids: requested} = parseArgs(argv, {});
  const ids = selectExtensions(requested);
  const listing = readPackages();
  for (const id of ids) {
    const records = await packExtension(id);
    // Replace what an earlier pack of this extension recorded.
    listing.packages = listing.packages.filter(record => record.id !== id).concat(records);
    for (const record of records) console.log(`Packed ${record.file} (${record.size} bytes)`);
  }
  listing.packages.sort((a, b) => (a.id + a.platform < b.id + b.platform ? -1 : 1));
  writeAtomic(packagesFile(), JSON.stringify(listing, null, 2) + '\n');
}
