// validate [id…]: check what `build` staged. Every staged package is checked
// structurally; with the Loom validator, language packs are also loaded the way the
// application loads them and their samples are colored.
import path from 'node:path';
import {DIST, EXTENSIONS, assert, selectExtensions, writeAtomic} from '../lib/util.mjs';
import {checkExtension} from '../lib/manifest.mjs';
import {hasRecipe, stageDirectory, stagedPlatforms} from '../lib/stage.mjs';
import {readRecipe} from '../lib/builders.mjs';
import {fetchValidator, findValidator, runValidator} from '../lib/validator.mjs';
import {parseArgs} from '../lib/cli.mjs';

export const usage = 'validate [id…] [--require-validator] [--offline]    Check staged packages, and color samples with loom-ext-validate';

export async function run(argv) {
  const {options, ids: requested} = parseArgs(argv, {'require-validator': 'boolean', offline: 'boolean'});
  const ids = selectExtensions(requested);
  let validator = findValidator();
  if (!validator && !options.offline) validator = await fetchValidator({offline: false}).catch(() => null);
  if (!validator) {
    assert(!options['require-validator'], 'loom-ext-validate is required but was not found: set LOOM_EXT_VALIDATE or fill tools/validator.lock');
    console.warn('loom-ext-validate was not found; only the structure of the packages is checked.');
  }
  const failures = [];
  for (const id of ids) {
    const platforms = stagedPlatforms(id, DIST);
    if (!platforms.length) { failures.push(`${id}: nothing is staged; run build first`); continue; }
    for (const platform of platforms) {
      const directory = stageDirectory(id, platform, DIST);
      const { problems } = checkExtension(directory, {built: true});
      if (problems.length) failures.push(`${id} (${platform}):\n  ${problems.join('\n  ')}`);
    }
    if (!validator || !hasRecipe(id)) continue;
    const recipe = readRecipe(id);
    if (recipe.kind !== 'language') continue;
    const samples = Object.entries(recipe.grammars).map(([grammar, definition]) => ({grammar, sample: path.join(EXTENSIONS, id, ...definition.sample.split('/'))}));
    const samplesFile = path.join(DIST, 'samples', `${id}.json`);
    writeAtomic(samplesFile, JSON.stringify(samples));
    try {
      const results = runValidator(validator, stageDirectory(id, platforms[0], DIST), samplesFile);
      for (const { grammar } of samples) {
        const found = results.find(result => result.grammar === grammar);
        if (!found || !(found.colored > 0)) failures.push(`${id}: the sample for ${grammar} is not colored`);
      }
    } catch (error) { failures.push(error.message); }
  }
  if (failures.length) throw new Error(failures.join('\n'));
  console.log(`Validated ${ids.length} extension(s)${validator ? ' with loom-ext-validate' : ''}.`);
}
