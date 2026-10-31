// The documents every published extension carries, written for the ones that are
// built from a recipe: README, changelog and an icon.
import {hash} from './util.mjs';

const hue = id => parseInt(hash(id).slice(0, 4), 16) % 360;

/** A rounded square in a color taken from the id, with the language's short name. */
export function iconSvg(id, text) {
  const h = hue(id);
  const size = text.length > 2 ? 22 : 28;
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><rect width="64" height="64" rx="14" fill="hsl(${h} 35% 16%)"/>` +
    `<text x="32" y="${size === 22 ? 40 : 42}" font-family="sans-serif" font-size="${size}" font-weight="700" text-anchor="middle" fill="hsl(${h} 80% 72%)">${text.replace(/&/g, '&amp;').replace(/</g, '&lt;')}</text></svg>\n`;
}

/** README.md, CHANGELOG.md and icon.svg for the recipe-built extension `id`. */
export function documentFiles(id, recipe, {license, badge}) {
  const {name, version} = recipe.extension;
  const grammars = Object.entries(recipe.grammars);
  const types = grammars.flatMap(([, grammar]) => grammar['file-types']).map(type => `\`${type}\``).join(', ');
  const readme = [
    `# ${name} Language`,
    '',
    recipe.extension.description,
    '',
    `- Grammars: ${grammars.map(([grammar]) => `\`${grammar}\``).join(', ')}`,
    `- File types: ${types || 'none'}`,
    `- License of the bundled grammar and queries: ${license}`,
    '',
    'The grammar is a Tree-sitter parser compiled to WebAssembly from pinned sources;',
    '`PROVENANCE.md` lists them and `licenses/` carries their licenses.',
    '',
    '## Change how it is colored',
    '',
    `Put your own \`${grammars[0][0]}/highlights.scm\` in Loom's queries folder. Start the file with`,
    '`; extends` to add rules to the ones here; without it your file replaces them. Use',
    '**Inspect Highlight at Cursor** to see which rule colors a place.',
    '',
  ].join('\n');
  const changelog = `# Changelog\n\n## ${version}\n\n- Built from the pinned sources listed in PROVENANCE.md.\n`;
  return {'README.md': readme, 'CHANGELOG.md': changelog, 'icon.svg': iconSvg(id, badge)};
}
