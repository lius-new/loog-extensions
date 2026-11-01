// The stage: `dist/stage/<platform>/<id>/` holds the unpacked files of each package
// taken in, so the index entry can be described from them.
import fs from 'node:fs';
import path from 'node:path';
import {DIST, removeUnder} from './util.mjs';

export const stageRoot = (dist = DIST) => path.join(dist, 'stage');

/** Platform folders under the stage that hold extension `id`. */
export function stagedPlatforms(id, dist = DIST) {
  const root = stageRoot(dist);
  if (!fs.existsSync(root)) return [];
  return fs.readdirSync(root, {withFileTypes: true})
    .filter(entry => entry.isDirectory() && fs.existsSync(path.join(root, entry.name, id, 'manifest.json')))
    .map(entry => entry.name).sort();
}

/** Clear everything staged for `id`, so a new intake cannot leave stale platforms behind. */
export function clearStage(id, dist = DIST) {
  const root = stageRoot(dist);
  if (!fs.existsSync(root)) return;
  for (const entry of fs.readdirSync(root, {withFileTypes: true})) {
    if (!entry.isDirectory()) continue;
    const target = path.join(root, entry.name, id);
    if (fs.existsSync(target)) removeUnder(root, target);
  }
}

export function stageDirectory(id, platform, dist = DIST) {
  return path.join(stageRoot(dist), platform, id);
}
