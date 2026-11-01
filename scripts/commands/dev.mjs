// dev <package…> [--to dir]: unpack finished packages into a folder Loom loads extensions
// from, to try them without publishing. Point Loom at the folder with LOOM_EXTENSIONS_DIR.
// Build the packages however the extension's project builds them.
import fs from 'node:fs';
import path from 'node:path';
import {WORK, assert, copyTree, removeUnder} from '../lib/util.mjs';
import {extractPackage} from '../lib/archive.mjs';
import {checkExtension} from '../lib/manifest.mjs';
import {ANY, hostPlatform} from '../lib/platform.mjs';
import {packageFiles, platformOf} from '../lib/intake.mjs';
import {parseArgs} from '../lib/cli.mjs';

export const usage = 'dev <package or folder…> [--to dir]    Unpack finished packages into a folder Loom loads extensions from';

export async function run(argv) {
  const {options, ids: paths} = parseArgs(argv, {to: 'string'});
  assert(paths.length, 'Name the .loomext packages, or the folders holding them');
  const target = options.to ?? process.env.LOOM_EXTENSIONS_DIR;
  assert(target, 'Say where to put them: --to <dir>, or set LOOM_EXTENSIONS_DIR');
  const destination = path.resolve(target);
  fs.mkdirSync(destination, {recursive: true});
  const host = hostPlatform(), scratch = path.join(WORK, 'dev');
  const chosen = new Map();
  for (const [number, file] of packageFiles(paths).entries()) {
    const directory = await extractPackage(file, path.join(scratch, String(number)));
    const {manifest, problems} = checkExtension(directory, {built: true});
    assert(!problems.length, `${path.basename(file)}:\n  ${problems.join('\n  ')}`);
    const platform = platformOf(file, manifest);
    // The package for this machine, or the one that runs anywhere.
    if (platform !== host && platform !== ANY) continue;
    if (!chosen.has(manifest.id) || platform === host) chosen.set(manifest.id, directory);
  }
  assert(chosen.size, `None of the packages is for ${host}`);
  for (const [id, directory] of chosen) {
    const output = path.join(destination, id);
    removeUnder(destination, output);
    copyTree(directory, output);
    console.log(`${id} -> ${output}`);
  }
  removeUnder(WORK, scratch);
}
