// Packs loog.theme.aura into dist/loog.theme.aura-<version>-any.loogext.
//   npm ci && node build.mjs
// The extension is complete as written: every file here except the build files below
// is part of the package. This project depends on nothing outside itself except what
// package.json names.
import fs from 'node:fs';
import path from 'node:path';
import * as tar from 'tar';

const BUILD_FILES = new Set(['build.mjs', 'package.json', 'package-lock.json', 'node_modules', 'dist', '.work']);

const here = import.meta.dirname;
const manifest = JSON.parse(fs.readFileSync(path.join(here, 'manifest.json'), 'utf8'));
const dist = path.join(here, 'dist');

// The package folder, named after the extension.
const folder = path.join(dist, 'package', manifest.id);
fs.rmSync(path.join(dist, 'package'), {recursive: true, force: true});
fs.mkdirSync(folder, {recursive: true});
for (const entry of fs.readdirSync(here)) if (!BUILD_FILES.has(entry)) fs.cpSync(path.join(here, entry), path.join(folder, entry), {recursive: true});

// A .loogext is a gzip tar whose one root folder is the extension id.
const output = path.join(dist, `${manifest.id}-${manifest.version}-any.loogext`);
const names = fs.readdirSync(folder, {recursive: true, withFileTypes: true}).filter(entry => entry.isFile())
  .map(entry => path.relative(folder, path.join(entry.parentPath, entry.name)).split(path.sep).join('/')).sort();
await tar.c({file: output, cwd: path.dirname(folder), gzip: {level: 9}, portable: true, mtime: new Date(0), noDirRecurse: true},
  [manifest.id, ...names.map(name => `${manifest.id}/${name}`)]);
console.log(`Wrote ${path.relative(here, output)}`);
