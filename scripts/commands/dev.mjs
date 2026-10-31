// dev [id…] --to dir: build and copy extensions into a folder Loom reads, to try them
// without publishing. Point Loom at the folder with LOOM_EXTENSIONS_DIR.
import fs from 'node:fs';
import path from 'node:path';
import {DIST, WORK, assert, copyTree, removeUnder, selectExtensions} from '../lib/util.mjs';
import {buildExtension} from '../lib/builders.mjs';
import {clearStage, stageDirectory, stagedPlatforms} from '../lib/stage.mjs';
import {ANY, hostPlatform} from '../lib/platform.mjs';
import {parseArgs} from '../lib/cli.mjs';

export const usage = 'dev [id…] [--to dir] [--offline]    Build and copy into a folder Loom loads extensions from';

export async function run(argv) {
  const {options, ids: requested} = parseArgs(argv, {to: 'string', offline: 'boolean'});
  const target = options.to ?? process.env.LOOM_EXTENSIONS_DIR;
  assert(target, 'Say where to put them: --to <dir>, or set LOOM_EXTENSIONS_DIR');
  const destination = path.resolve(target);
  fs.mkdirSync(destination, {recursive: true});
  const host = hostPlatform();
  for (const id of selectExtensions(requested)) {
    clearStage(id, DIST);
    await buildExtension(id, {dist: DIST, work: WORK, offline: Boolean(options.offline)});
    const platforms = stagedPlatforms(id, DIST);
    const platform = platforms.includes(host) ? host : platforms.includes(ANY) ? ANY : null;
    assert(platform, `${id} has no package for ${host}`);
    const output = path.join(destination, id);
    removeUnder(destination, output);
    copyTree(stageDirectory(id, platform, DIST), output);
    console.log(`${id} -> ${output}`);
  }
}
