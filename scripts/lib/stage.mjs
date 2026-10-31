// Staging: turn each extension into the folders that get packed.
//
// `dist/stage/<platform>/<id>/` holds one package's worth of files. A hand-written
// extension is copied as it is, once, as platform `any`. One with a `recipe.toml` is
// built by the builder for its kind (see lib/builders.mjs), which says which
// platforms it makes.
import fs from 'node:fs';
import path from 'node:path';
import {DIST, EXTENSIONS, assert, copyTree, removeUnder} from './util.mjs';
import {checkExtension} from './manifest.mjs';
import {ANY} from './platform.mjs';

export const stageRoot = (dist = DIST) => path.join(dist, 'stage');
export const hasRecipe = (id, root = EXTENSIONS) => fs.existsSync(path.join(root, id, 'recipe.toml'));

/** Platform folders under the stage that hold extension `id`. */
export function stagedPlatforms(id, dist = DIST) {
  const root = stageRoot(dist);
  if (!fs.existsSync(root)) return [];
  return fs.readdirSync(root, {withFileTypes: true})
    .filter(entry => entry.isDirectory() && fs.existsSync(path.join(root, entry.name, id, 'manifest.json')))
    .map(entry => entry.name).sort();
}

/** Copy a hand-written extension into the stage, and check what was staged. */
export function stagePlain(id, {root = EXTENSIONS, dist = DIST} = {}) {
  const target = path.join(stageRoot(dist), ANY, id);
  fs.mkdirSync(path.dirname(target), {recursive: true});
  removeUnder(stageRoot(dist), target);
  copyTree(path.join(root, id), target);
  const {problems} = checkExtension(target, {built: true});
  assert(!problems.length, `${id}:\n  ${problems.join('\n  ')}`);
  return [ANY];
}

/** Clear everything staged for `id`, so a rebuild cannot leave stale platforms behind. */
export function clearStage(id, dist = DIST) {
  for (const platform of stagedPlatforms(id, dist)) removeUnder(stageRoot(dist), path.join(stageRoot(dist), platform, id));
}

export function stageDirectory(id, platform, dist = DIST) {
  return path.join(stageRoot(dist), platform, id);
}
