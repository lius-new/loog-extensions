// Builds loom.lang.bash into dist/loom.lang.bash-<version>-any.loomext.
//   npm ci && node build.mjs
// The package is the files named below plus what is compiled and fetched here: the
// pinned tree-sitter-bash grammar as WebAssembly, its highlight query and its license.
// This project depends on nothing outside itself except what package.json names.
import fs from 'node:fs';
import path from 'node:path';
import {execFileSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import * as tar from 'tar';

const SOURCE = {
  repository: 'tree-sitter/tree-sitter-bash',
  revision: 'a06c2e4415e9bc0346c6b86d401879ffb44058f7',
  sha256: '879e8951ea2cc82455407e3eda0293319657ec53e83191e0fb5a67430d10d804',
};
const FILES = ['manifest.json', 'tree-sitter.json', 'README.md', 'CHANGELOG.md', 'icon.svg'];

const here = import.meta.dirname;
const manifest = JSON.parse(fs.readFileSync(path.join(here, 'manifest.json'), 'utf8'));
const work = path.join(here, '.work'), dist = path.join(here, 'dist');
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');

// The pinned source archive: downloaded once, checked, unpacked.
const archive = path.join(work, `${SOURCE.sha256}.tar.gz`);
if (!(fs.existsSync(archive) && sha256(fs.readFileSync(archive)) === SOURCE.sha256)) {
  const url = `https://codeload.github.com/${SOURCE.repository}/tar.gz/${SOURCE.revision}`;
  const response = await fetch(url, {signal: AbortSignal.timeout(600000)});
  if (!response.ok) throw new Error(`${url}: HTTP ${response.status}`);
  const bytes = Buffer.from(await response.arrayBuffer());
  if (sha256(bytes) !== SOURCE.sha256) throw new Error(`${url}: the download does not match its pinned SHA-256`);
  fs.mkdirSync(work, {recursive: true});
  fs.writeFileSync(archive + '.partial', bytes);
  fs.renameSync(archive + '.partial', archive);
}
const source = path.join(work, 'source');
fs.rmSync(source, {recursive: true, force: true});
fs.mkdirSync(source, {recursive: true});
await tar.x({file: archive, cwd: source, strip: 1});

// The package folder, named after the extension.
const folder = path.join(dist, 'package', manifest.id);
fs.rmSync(path.join(dist, 'package'), {recursive: true, force: true});
fs.mkdirSync(path.join(folder, 'queries'), {recursive: true});
for (const file of FILES) fs.copyFileSync(path.join(here, file), path.join(folder, file));

// Grammar to WebAssembly with the Tree-sitter CLI from node_modules.
execFileSync(process.execPath, [path.join(here, 'node_modules', 'tree-sitter-cli', 'cli.js'), 'build', '--wasm',
  '--output', path.join(folder, 'tree-sitter-bash.wasm'), source], {stdio: 'inherit'});
fs.copyFileSync(path.join(source, 'queries', 'highlights.scm'), path.join(folder, 'queries', 'highlights.scm'));

const license = fs.readFileSync(path.join(source, 'LICENSE'), 'utf8');
if (!/Permission is hereby granted, free of charge, to any person obtaining a copy/i.test(license.replace(/\s+/g, ' '))) {
  throw new Error(`the upstream license is no longer ${manifest.license}; update the manifest`);
}
fs.writeFileSync(path.join(folder, 'LICENSE'), license);
fs.writeFileSync(path.join(folder, 'PROVENANCE.md'), `# Bash\n\n- https://github.com/${SOURCE.repository} @ ${SOURCE.revision}\n`);

// A .loomext is a gzip tar whose one root folder is the extension id.
const output = path.join(dist, `${manifest.id}-${manifest.version}-any.loomext`);
const names = fs.readdirSync(folder, {recursive: true, withFileTypes: true}).filter(entry => entry.isFile())
  .map(entry => path.relative(folder, path.join(entry.parentPath, entry.name)).split(path.sep).join('/')).sort();
await tar.c({file: output, cwd: path.dirname(folder), gzip: {level: 9}, portable: true, mtime: new Date(0), noDirRecurse: true},
  [manifest.id, ...names.map(name => `${manifest.id}/${name}`)]);
console.log(`Wrote ${path.relative(here, output)}`);
