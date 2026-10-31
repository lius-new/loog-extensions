// check [id…]: everything that can be verified without building or downloading.
import fs from 'node:fs';
import path from 'node:path';
import {EXTENSIONS, ROOT, readJson, selectExtensions} from '../lib/util.mjs';
import {checkExtension, kindOf} from '../lib/manifest.mjs';
import {checkRecipe} from '../lib/builders.mjs';
import {hasRecipe} from '../lib/stage.mjs';
import {loadConfig} from '../lib/config.mjs';
import {parseArgs} from '../lib/cli.mjs';

export const usage = 'check [id…]    Check manifests, recipes and the repository settings';

/** Problems that involve more than one extension. */
export function checkRepository(ids, config, root = EXTENSIONS) {
  const problems = [];
  for (const id of config.bundled) if (!fs.existsSync(path.join(root, id))) problems.push(`loom-ext.toml: bundled extension ${id} does not exist`);
  const claims = new Map();
  for (const id of ids) {
    const file = path.join(root, id, 'manifest.json');
    if (!fs.existsSync(file)) continue;
    let manifest;
    try { manifest = readJson(file); } catch { continue; }
    for (const resource of manifest.contributes?.languages ?? []) {
      const pack = path.join(root, id, ...String(resource.path).split('/'));
      if (!fs.existsSync(pack)) continue;
      try {
        for (const grammar of readJson(pack).grammars ?? []) {
          for (const suffix of grammar['file-types'] ?? []) {
            const key = suffix.toLowerCase();
            if (claims.has(key) && claims.get(key) !== id) problems.push(`file type ${suffix} is claimed by both ${claims.get(key)} and ${id}`);
            claims.set(key, id);
          }
        }
      } catch { /* reported by the extension's own check */ }
    }
  }
  return problems;
}

export async function run(argv) {
  const {ids: requested} = parseArgs(argv, {});
  const config = loadConfig();
  const ids = selectExtensions(requested);
  let failed = 0;
  const report = (id, problems) => {
    if (!problems.length) return;
    failed += problems.length;
    console.error(`${id}:\n  ${problems.join('\n  ')}`);
  };
  for (const id of ids) {
    if (hasRecipe(id)) {
      try { report(id, await checkRecipe(id)); } catch (error) { report(id, [error.message]); }
    } else {
      report(id, checkExtension(path.join(EXTENSIONS, id)).problems);
    }
    if (!kindOf(id)) report(id, [`the name does not follow the naming scheme (${['loom.lang.*', 'loom.lsp.*', 'loom.theme.*', 'loom.queries.*', 'loom.runtime.*'].join(', ')})`]);
  }
  if (!requested.length) report('(repository)', checkRepository(ids, config));
  if (failed) throw new Error(`${failed} problem(s) in ${path.relative(ROOT, EXTENSIONS) || 'extensions'}`);
  console.log(`Checked ${ids.length} extension(s): no problems.`);
}
