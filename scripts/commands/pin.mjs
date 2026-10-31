// pin [id…] [--refresh]: resolve and write the pins of server and runtime extensions.
import fs from 'node:fs';
import path from 'node:path';
import {EXTENSIONS, selectExtensions} from '../lib/util.mjs';
import {readRecipe} from '../lib/builders.mjs';
import {hasRecipe} from '../lib/stage.mjs';
import {pinNpm, pinPack, readPins, writePins} from '../lib/pin.mjs';
import {parseArgs} from '../lib/cli.mjs';

export const usage = 'pin [id…] [--refresh]    Pin upstream archives and lockfiles of servers and runtimes (needs the network)';

export async function run(argv) {
  const {options, ids: requested} = parseArgs(argv, {refresh: 'boolean'});
  const ids = selectExtensions(requested).filter(id => hasRecipe(id) && ['server', 'runtime'].includes(readRecipe(id).kind));
  for (const id of ids) {
    const recipe = readRecipe(id);
    if (recipe.pack.kind === 'npm') {
      const locked = fs.existsSync(path.join(EXTENSIONS, id, 'npm', 'package-lock.json'));
      if (options.refresh || !locked) { console.log(`locking ${id}`); pinNpm(id, recipe); }
      continue;
    }
    writePins(id, await pinPack(id, recipe, readPins(id), {refresh: Boolean(options.refresh)}));
  }
  console.log(`Pinned ${ids.length} extension(s).`);
}
