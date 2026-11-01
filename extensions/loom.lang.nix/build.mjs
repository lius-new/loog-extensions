// Builds loom.lang.nix into dist/loom.lang.nix-<version>-any.loomext.
//   npm ci && node build.mjs
// The grammars below are compiled to WebAssembly with the Tree-sitter CLI from
// node_modules; their highlight queries and licenses come from the same pinned sources.
// This project depends on nothing outside itself except what package.json names.
import fs from 'node:fs';
import path from 'node:path';
import {execFileSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import * as tar from 'tar';

const CONFIG = {
  "files": [
    "manifest.json",
    "tree-sitter.json",
    "README.md",
    "CHANGELOG.md",
    "icon.svg"
  ],
  "sources": {
    "nix": {
      "licenses": [
        "LICENSE"
      ],
      "repository": "https://github.com/numtide/tree-sitter-nix",
      "revision": "70f34e95e30b7ebcc40815d4385d68576c4a15cd",
      "sha256": "e3de0d2095ddb28ba5929f2be8d7d76733412483ce93bdd14da908fa29ed95fe"
    }
  },
  "grammars": {
    "nix": {
      "source": "nix",
      "path": ".",
      "highlights": [
        {
          "path": "queries/highlights.scm"
        }
      ],
      "captureMap": {
        "spell": "_spell",
        "boolean": "constant.builtin.boolean"
      }
    }
  },
  "patches": []
};

const here = import.meta.dirname;
const manifest = JSON.parse(fs.readFileSync(path.join(here, 'manifest.json'), 'utf8'));
const work = path.join(here, '.work'), dist = path.join(here, 'dist');
const cli = path.join(here, 'node_modules', 'tree-sitter-cli', 'cli.js');
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
const run = (args, options = {}) => execFileSync(process.execPath, [cli, ...args], {stdio: 'inherit', ...options});

// Pinned sources: downloaded once, checked, unpacked.
const prepared = new Map();
async function source(name) {
  if (prepared.has(name)) return prepared.get(name);
  const spec = CONFIG.sources[name];
  let directory;
  if (spec.directory) directory = path.join(here, ...spec.directory.split('/'));
  else {
    const url = spec.url ?? `https://codeload.github.com/${spec.repository.replace('https://github.com/', '')}/tar.gz/${spec.revision}`;
    const archive = path.join(work, 'downloads', `${spec.sha256}.tar.gz`);
    if (!(fs.existsSync(archive) && sha256(fs.readFileSync(archive)) === spec.sha256)) {
      const response = await fetch(url, {signal: AbortSignal.timeout(600000)});
      if (!response.ok) throw new Error(`${url}: HTTP ${response.status}`);
      const bytes = Buffer.from(await response.arrayBuffer());
      if (sha256(bytes) !== spec.sha256) throw new Error(`${url}: the download does not match its pinned SHA-256`);
      fs.mkdirSync(path.dirname(archive), {recursive: true});
      fs.writeFileSync(archive + '.partial', bytes);
      fs.renameSync(archive + '.partial', archive);
    }
    const content = path.join(work, 'sources', spec.sha256);
    if (!fs.existsSync(path.join(content, '.complete'))) {
      fs.rmSync(content, {recursive: true, force: true});
      fs.mkdirSync(content, {recursive: true});
      await tar.x({file: archive, cwd: content});
      fs.writeFileSync(path.join(content, '.complete'), spec.sha256);
    }
    const entries = fs.readdirSync(content, {withFileTypes: true}).filter(entry => entry.name !== '.complete');
    directory = entries.length === 1 && entries[0].isDirectory() ? path.join(content, entries[0].name) : content;
  }
  prepared.set(name, directory);
  return directory;
}

// Rename captures Loom knows under another name; comments and strings are left alone.
const rewriteCaptures = (text, mapping = {}) => text.replace(/;[^\n]*|"(?:\\.|[^"\\])*"|@([\w.-]+)/g,
  (token, name) => (name && Object.hasOwn(mapping, name) ? `@${mapping[name]}` : token));

function licenseOf(text) {
  const t = text.replace(/\s+/g, ' ');
  if (/Apache License,? Version 2\.0/i.test(t)) return 'Apache-2.0';
  if (/Mozilla Public License,? (Version|v\.?) ?2\.0/i.test(t)) return 'MPL-2.0';
  if (/GNU (Lesser|Library) General Public License/i.test(t) || /GNU General Public License/i.test(t)) return null;
  if (/This is free and unencumbered software released into the public domain/i.test(t)) return 'Unlicense';
  if (/Permission is hereby granted, free of charge, to any person obtaining a copy/i.test(t)) return 'MIT';
  if (/Permission to use, copy, modify, and\/or distribute this software for any purpose/i.test(t)) return 'ISC';
  if (/Redistribution and use in source and binary forms/i.test(t)) return /Neither the name|may be used to endorse/i.test(t) ? 'BSD-3-Clause' : 'BSD-2-Clause';
  return null;
}

// The package folder, named after the extension.
const folder = path.join(dist, 'package', manifest.id);
fs.rmSync(path.join(dist, 'package'), {recursive: true, force: true});
fs.mkdirSync(folder, {recursive: true});
for (const file of CONFIG.files) {
  fs.mkdirSync(path.dirname(path.join(folder, file)), {recursive: true});
  fs.copyFileSync(path.join(here, file), path.join(folder, file));
}

const used = new Set();
async function compile(name, grammar, directory, output) {
  run(['build', '--wasm', '--output', output, directory]);
}

for (const [name, grammar] of Object.entries(CONFIG.grammars)) {
  used.add(grammar.source);
  const directory = path.join(await source(grammar.source), ...grammar.path.split('/'));
  if (!fs.existsSync(path.join(directory, 'src', 'parser.c'))) throw new Error(`${name}: the source has no src/parser.c`);
  const output = path.join(folder, `tree-sitter-${name}.wasm`);
  await compile(name, grammar, directory, output);

  for (const kind of ['highlights', 'injections']) {
    const references = grammar[kind] ?? [];
    if (!references.length) continue;
    const texts = [];
    for (const reference of references) {
      used.add(reference.source ?? grammar.source);
      texts.push(fs.readFileSync(path.join(await source(reference.source ?? grammar.source), ...reference.path.split('/')), 'utf8'));
    }
    const file = path.join(folder, 'queries', name, `${kind}.scm`);
    fs.mkdirSync(path.dirname(file), {recursive: true});
    fs.writeFileSync(file, rewriteCaptures(texts.join('\n'), kind === 'highlights' ? grammar.captureMap : {}));
  }
}
for (const patch of CONFIG.patches) {
  // Run inside the package folder: the patch names files relative to it.
  execFileSync('git', ['apply', '--unsafe-paths', '--whitespace=error', path.join(here, ...patch.split('/'))], {cwd: folder, stdio: 'inherit'});
}

// Licenses of every source that went in; the manifest's license is what they say.
const found = new Set(), provenance = [];
for (const name of [...used].sort()) {
  const spec = CONFIG.sources[name];
  const directory = await source(name);
  for (const [index, file] of spec.licenses.entries()) {
    const text = fs.readFileSync(path.join(directory, ...file.split('/')), 'utf8');
    found.add(licenseOf(text) ?? 'NOASSERTION');
    fs.mkdirSync(path.join(folder, 'licenses'), {recursive: true});
    fs.writeFileSync(path.join(folder, 'licenses', `${name}-${index}-${path.basename(file)}`), text);
  }
  provenance.push(`- ${name}: ${spec.directory ? `${manifest.id} / ${spec.directory}` : `${spec.repository} @ ${spec.revision}`}\n`);
}
const license = found.has('NOASSERTION') ? 'NOASSERTION' : [...found].sort().join(' AND ');
if (license !== manifest.license) throw new Error(`the sources are licensed ${license}, the manifest says ${manifest.license}; update the manifest`);
fs.writeFileSync(path.join(folder, 'PROVENANCE.md'), `# ${manifest.name}\n\nBuilt from pinned sources with the Tree-sitter CLI.\n\n${provenance.join('')}`);

// A .loomext is a gzip tar whose one root folder is the extension id.
const output = path.join(dist, `${manifest.id}-${manifest.version}-any.loomext`);
const names = fs.readdirSync(folder, {recursive: true, withFileTypes: true}).filter(entry => entry.isFile())
  .map(entry => path.relative(folder, path.join(entry.parentPath, entry.name)).split(path.sep).join('/')).sort();
await tar.c({file: output, cwd: path.dirname(folder), gzip: {level: 9}, portable: true, mtime: new Date(0), noDirRecurse: true},
  [manifest.id, ...names.map(name => `${manifest.id}/${name}`)]);
console.log(`Wrote ${path.relative(here, output)}`);
