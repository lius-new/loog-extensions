// Chooses how an extension is built. A folder without a `recipe.toml` is copied;
// a recipe names its kind and the builder for that kind does the rest.
import path from 'node:path';
import {EXTENSIONS, assert, readToml} from './util.mjs';
import {hasRecipe, stagePlain} from './stage.mjs';

export const KINDS = {
  language: './build-language.mjs',
  server: './build-server.mjs',
  runtime: './build-server.mjs',
};

export function readRecipe(id, root = EXTENSIONS) {
  const recipe = readToml(path.join(root, id, 'recipe.toml'));
  assert(recipe.schema === 1, `${id}: recipe schema must be 1`);
  assert(KINDS[recipe.kind], `${id}: unknown recipe kind ${JSON.stringify(recipe.kind)}`);
  return recipe;
}

/** Problems with a recipe that can be found without building or downloading. */
export async function checkRecipe(id, options = {}) {
  const recipe = readRecipe(id, options.root);
  const builder = await import(KINDS[recipe.kind]);
  return builder.checkRecipe(id, recipe, options);
}

/** Build `id` into the stage; returns the platforms it made. */
export async function buildExtension(id, options = {}) {
  if (!hasRecipe(id, options.root)) return stagePlain(id, options);
  const recipe = readRecipe(id, options.root);
  const builder = await import(KINDS[recipe.kind]);
  return builder.build(id, recipe, options);
}
