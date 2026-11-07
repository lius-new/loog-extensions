// Builds loog.lsp.yaml-language-server into dist/loog.lsp.yaml-language-server-<version>-any.loogext.
//   npm ci && node build.mjs
// The server's own packages are in payload/ (package.json and its lock file, which
// pins every dependency by integrity hash). They are installed without running
// any lifecycle script. This project depends on nothing outside itself except what
// package.json names.
import fs from 'node:fs';
import path from 'node:path';
import {execFileSync} from 'node:child_process';
import * as tar from 'tar';

const FILES = ['manifest.json', 'README.md', 'CHANGELOG.md', 'icon.svg'];

const here = import.meta.dirname;
const manifest = JSON.parse(fs.readFileSync(path.join(here, 'manifest.json'), 'utf8'));
const work = path.join(here, '.work'), dist = path.join(here, 'dist');

// The package folder, named after the extension.
const folder = path.join(dist, 'package', manifest.id);
fs.rmSync(path.join(dist, 'package'), {recursive: true, force: true});
fs.mkdirSync(folder, {recursive: true});
for (const file of FILES) fs.copyFileSync(path.join(here, file), path.join(folder, file));

// The lock file is the only source of versions.
const install = path.join(work, 'payload');
fs.rmSync(install, {recursive: true, force: true});
fs.mkdirSync(install, {recursive: true});
for (const name of ['package.json', 'package-lock.json']) fs.copyFileSync(path.join(here, 'payload', name), path.join(install, name));
const windows = process.platform === 'win32';
execFileSync(windows ? 'npm.cmd' : 'npm', ['ci', '--ignore-scripts', '--omit=dev', '--no-audit', '--no-fund', '--no-bin-links'],
  {cwd: install, stdio: 'inherit', shell: windows, env: {...process.env, npm_config_update_notifier: 'false'}});
fs.cpSync(path.join(install, 'node_modules'), path.join(folder, 'node_modules'), {recursive: true});

const locked = JSON.parse(fs.readFileSync(path.join(install, 'package-lock.json'), 'utf8')).packages;
const list = Object.entries(locked).filter(([name, entry]) => name && !entry.link)
  .map(([name, entry]) => `- ${name.replace(/^node_modules\//, '')} ${entry.version} ${entry.integrity}`).sort();
fs.writeFileSync(path.join(folder, 'PROVENANCE.md'), `# ${manifest.name}\n\nPackages (${list.length}), locked by integrity:\n${list.join('\n')}\n`);

// A .loogext is a gzip tar whose one root folder is the extension id.
const output = path.join(dist, `${manifest.id}-${manifest.version}-any.loogext`);
const names = fs.readdirSync(folder, {recursive: true, withFileTypes: true}).filter(entry => entry.isFile())
  .map(entry => path.relative(folder, path.join(entry.parentPath, entry.name)).split(path.sep).join('/')).sort();
await tar.c({file: output, cwd: path.dirname(folder), gzip: {level: 9}, portable: true, mtime: new Date(0), noDirRecurse: true},
  [manifest.id, ...names.map(name => `${manifest.id}/${name}`)]);
console.log(`Wrote ${path.relative(here, output)}`);
