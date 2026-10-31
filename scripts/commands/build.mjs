// build [id…]: stage extensions into dist/stage, building the ones that have a recipe.
import path from 'node:path';
import {DIST, WORK, selectExtensions} from '../lib/util.mjs';
import {buildExtension} from '../lib/builders.mjs';
import {clearStage} from '../lib/stage.mjs';
import {parseArgs} from '../lib/cli.mjs';

export const usage = 'build [id…] [--offline] [--work dir]    Build into dist/stage (no network with --offline)';

export async function run(argv) {
  const {options, ids: requested} = parseArgs(argv, {offline: 'boolean', work: 'string'});
  const ids = selectExtensions(requested);
  const context = {dist: DIST, work: options.work ? path.resolve(options.work) : WORK, offline: Boolean(options.offline)};
  const failures = [];
  for (const id of ids) {
    try {
      clearStage(id, context.dist);
      const platforms = await buildExtension(id, context);
      console.log(`Built ${id} (${platforms.join(', ')})`);
    } catch (error) {
      failures.push(id);
      console.error(`${id}: ${error.message}`);
    }
  }
  if (failures.length) throw new Error(`${failures.length} extension(s) failed: ${failures.join(', ')}`);
}
