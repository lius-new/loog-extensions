// Builds a language extension from its recipe: fetch each grammar's pinned source,
// compile it to WebAssembly, collect the queries, and write the files that make it
// an extension (manifest, README, changelog, icon, provenance and licenses).
import fs from 'node:fs';
import path from 'node:path';
import {EXTENSIONS, HASH, WORK, DIST, assert, canonical, fileHash, files, hash, inside,
  relative, removeUnder, run, writeAtomic} from './util.mjs';
import {extractArchive, fetchPinned, oneRoot} from './fetch.mjs';
import {loadToolchain, prepareTools, toolchainFile} from './toolchain.mjs';
import {checkExtension, validId} from './manifest.mjs';
import {validVersion} from './semver.mjs';
import {ANY} from './platform.mjs';
import {loadConfig} from './config.mjs';
import {stageDirectory, stageRoot} from './stage.mjs';
import {documentFiles} from './documents.mjs';

const GRAMMAR = /^[a-z][a-z0-9_]*$/;
const SOURCE = /^[a-z][a-z0-9_-]*$/;

/** Problems with a language recipe that need no network. */
export function checkRecipe(id, recipe, {root = EXTENSIONS} = {}) {
  const problems = [];
  const dir = path.join(root, id);
  const bad = message => problems.push(message);
  if (!id.startsWith('loom.lang.') || !validId(id)) bad('a language extension is named loom.lang.<name>');
  const extension = recipe.extension ?? {};
  if (typeof extension.name !== 'string' || !extension.name.trim() || extension.name.length > 50) bad('extension.name is required (up to 50 characters)');
  if (!validVersion(extension.version ?? '')) bad('extension.version must be a version such as 1.0.0');
  if (typeof extension.description !== 'string' || !extension.description.trim() || extension.description.length > 200) bad('extension.description is required (up to 200 characters)');
  for (const item of extension.recommends ?? []) if (!validId(item) || !item.startsWith('loom.lsp.')) bad(`extension.recommends: ${item} is not a language server extension`);
  const sources = recipe.sources ?? {};
  for (const [name, source] of Object.entries(sources)) {
    if (!SOURCE.test(name)) bad(`source ${name}: invalid name`);
    if (Boolean(source.directory) === Boolean(source.repository)) { bad(`source ${name}: needs either directory or repository`); continue; }
    if (source.directory) {
      try { inside(dir, source.directory); } catch (error) { bad(`source ${name}: ${error.message}`); }
    } else {
      if (!/^https:\/\/github\.com\/[\w.-]+\/[\w.-]+$/.test(source.repository)) bad(`source ${name}: expected a GitHub repository URL`);
      if (!/^[a-f0-9]{40}$/.test(source.revision ?? '')) bad(`source ${name}: pin a complete commit SHA`);
      if (!HASH.test(source.sha256 ?? '')) bad(`source ${name}: pin the archive SHA-256`);
      if (source.url && new URL(source.url).protocol !== 'https:') bad(`source ${name}: the archive URL must be https`);
    }
    if (!Array.isArray(source.licenses) || !source.licenses.length) bad(`source ${name}: list its license files`);
    for (const license of source.licenses ?? []) { try { relative(license); } catch (error) { bad(`source ${name}: ${error.message}`); } }
    for (const dependency of source.dependencies ?? []) {
      if (!sources[dependency.source]) bad(`source ${name}: unknown dependency ${dependency.source}`);
      try { relative(dependency.path); } catch (error) { bad(`source ${name}: ${error.message}`); }
    }
  }
  const grammars = recipe.grammars ?? {};
  if (!Object.keys(grammars).length) bad('the recipe has no grammars');
  for (const [name, grammar] of Object.entries(grammars)) {
    const here = message => bad(`grammar ${name}: ${message}`);
    if (!GRAMMAR.test(name) || name === 'plaintext') here('invalid name');
    if (!sources[grammar.source]) here(`unknown source ${grammar.source}`);
    if (grammar.path && grammar.path !== '.') { try { relative(grammar.path); } catch (error) { here(error.message); } }
    if (grammar.prebuilt) { try { relative(grammar.prebuilt); } catch (error) { here(error.message); } if (grammar.generate) here('cannot generate and use a prebuilt module'); }
    for (const support of grammar.support ?? []) {
      if (!sources[support.source] || grammar.optimize !== false) here('support sources need the WASI driver (optimize = false)');
      if (!String(support.path ?? '').endsWith('.c')) here('support sources are C files');
    }
    for (const [from, to] of Object.entries(grammar['capture-map'] ?? {})) if (!/^[\w.-]+$/.test(from) || !/^[\w.-]+$/.test(to)) here('invalid capture adaptation');
    if (!Array.isArray(grammar['file-types']) || grammar['file-types'].some(item => typeof item !== 'string' || !item || /[\\/\0]/.test(item))) here('file-types must list file name suffixes');
    if (!Array.isArray(grammar.highlights) || !grammar.highlights.length) here('missing highlights');
    for (const kind of ['highlights', 'injections']) {
      for (const query of grammar[kind] ?? []) {
        if (!sources[query.source ?? grammar.source]) here(`unknown query source ${query.source}`);
        try { relative(query.path); } catch (error) { here(error.message); }
      }
    }
    try { if (!fs.statSync(inside(dir, grammar.sample)).isFile()) here('the sample is not a file'); } catch { here(`missing sample ${grammar.sample}`); }
  }
  for (const patch of recipe.patches ?? []) { try { inside(dir, patch); } catch { bad(`missing patch ${patch}`); } }
  return problems;
}

export function sourcePreparer(sources, {work = WORK, offline = false, extDir} = {}) {
  const promises = new Map();
  const prepare = (name, ancestors = []) => {
    assert(!ancestors.includes(name), `Cyclic source dependencies: ${[...ancestors, name].join(' -> ')}`);
    if (promises.has(name)) return promises.get(name);
    const promise = (async () => {
      const spec = sources[name];
      let directory;
      if (spec.directory) directory = inside(extDir, spec.directory);
      else {
        const repository = spec.repository.slice('https://github.com/'.length);
        const url = spec.url || `https://codeload.github.com/${repository}/tar.gz/${spec.revision}`;
        const archive = await fetchPinned(url, spec.sha256, path.join(work, 'downloads', `${spec.sha256}.tar.gz`), {offline});
        const base = path.join(work, 'sources', spec.sha256), content = path.join(base, 'content');
        if (!fs.existsSync(path.join(base, 'complete'))) {
          removeUnder(work, content);
          await extractArchive(archive, content);
          fs.writeFileSync(path.join(base, 'complete'), spec.sha256);
        }
        directory = oneRoot(content);
      }
      if (spec.dependencies?.length) {
        const assembled = path.join(work, 'prepared-sources', `${name}-${hash(JSON.stringify(spec))}`);
        removeUnder(work, assembled);
        fs.cpSync(directory, assembled, {recursive: true, dereference: true});
        for (const dependency of spec.dependencies) {
          const prepared = await prepare(dependency.source, [...ancestors, name]);
          const target = path.join(assembled, relative(dependency.path));
          assert(!fs.existsSync(target), `Dependency destination already exists: ${target}`);
          fs.cpSync(prepared, target, {recursive: true, dereference: true});
        }
        directory = assembled;
      }
      return directory;
    })();
    promises.set(name, promise);
    return promise;
  };
  return prepare;
}

/** What a compiled module depends on, so an unchanged grammar is not compiled again. */
export function moduleFingerprint(recipe, tools, name, extDir) {
  const sources = {};
  const visit = source => {
    if (sources[source]) return;
    const spec = recipe.sources[source];
    sources[source] = {...spec};
    if (spec.directory) {
      const directory = inside(extDir, spec.directory);
      sources[source].files = Object.fromEntries(files(directory).map(file => [file, fileHash(path.join(directory, file))]));
    }
    for (const dependency of spec.dependencies ?? []) visit(dependency.source);
  };
  const grammar = recipe.grammars[name];
  visit(grammar.source);
  for (const support of grammar.support ?? []) visit(support.source);
  return hash(JSON.stringify(canonical({
    name, source: grammar.source, path: grammar.path || '.', generate: Boolean(grammar.generate), prebuilt: grammar.prebuilt || null,
    ...(grammar.optimize === false ? {optimize: false} : {}), ...(grammar.support?.length ? {support: grammar.support} : {}),
    tools, sources,
  })));
}

/** Rename captures the application knows under another name. Comments and strings are left alone. */
export function rewriteCaptures(text, mapping = {}) {
  return text.replace(/;[^\n]*|"(?:\\.|[^"\\])*"|@([\w.-]+)/g, (token, name) => (name && Object.hasOwn(mapping, name) ? `@${mapping[name]}` : token));
}

export function applyQueryPatch(directory, file) {
  const patch = fs.readFileSync(file, 'utf8');
  const paths = [...patch.matchAll(/^(?:---|\+\+\+) ([^\t\n]+)(?:\t[^\n]*)?$/gm)].map(match => match[1]);
  assert(paths.length && paths.every(name => {
    if (name === '/dev/null') return true;
    const value = name.replace(/^[ab]\//, '');
    relative(value);
    return value.startsWith('queries/') && value.endsWith('.scm');
  }), 'Patches may only modify package queries');
  run('git', ['apply', '--unsafe-paths', '--whitespace=error', `--directory=${directory.replaceAll('\\', '/')}`, file]);
}

/** An SPDX identifier for a license file, or null when it is not one of the common ones. */
export function detectLicense(text) {
  const t = text.replace(/\s+/g, ' ');
  if (/Apache License,? Version 2\.0/i.test(t)) return 'Apache-2.0';
  if (/Mozilla Public License,? (Version|v\.?) ?2\.0/i.test(t)) return 'MPL-2.0';
  if (/GNU (Lesser|Library) General Public License/i.test(t)) return null;
  if (/GNU General Public License/i.test(t)) return null;
  if (/This is free and unencumbered software released into the public domain/i.test(t)) return 'Unlicense';
  if (/Permission is hereby granted, free of charge, to any person obtaining a copy/i.test(t)) return 'MIT';
  if (/Permission to use, copy, modify, and\/or distribute this software for any purpose/i.test(t)) return 'ISC';
  if (/Redistribution and use in source and binary forms/i.test(t)) return /Neither the name|may be used to endorse/i.test(t) ? 'BSD-3-Clause' : 'BSD-2-Clause';
  return null;
}

/** One expression for all the licenses a package carries; `NOASSERTION` when any is unrecognised. */
export function licenseExpression(texts) {
  const found = new Set();
  for (const text of texts) {
    const id = detectLicense(text);
    if (!id) return 'NOASSERTION';
    found.add(id);
  }
  return found.size ? [...found].sort().join(' AND ') : 'NOASSERTION';
}

const badge = name => name.replace(/[^A-Za-z0-9+#]/g, '').slice(0, 3) || 'L';

export function languageManifest(id, recipe, {repository, license, grammars}) {
  const extension = recipe.extension;
  const keywords = [...new Set([extension.name.toLowerCase(), 'syntax', 'tree-sitter', ...grammars])].filter(word => word.length <= 30).slice(0, 10);
  return {
    schema: 1, id, version: extension.version, name: `${extension.name} Language`, publisher: 'Loom',
    description: extension.description, categories: ['Languages'], keywords, license,
    repository: `https://github.com/${repository}`, icon: 'icon.svg', readme: 'README.md', changelog: 'CHANGELOG.md',
    engines: {loom: extension.engines ?? '>=0.1.8 <0.3'},
    ...(extension.recommends?.length ? {recommends: extension.recommends} : {}),
    contributes: {languages: [{id: `${id}.grammar`, path: 'tree-sitter.json'}]},
  };
}

/** Build `id` into the stage. Returns the platforms made. */
export async function build(id, recipe, {dist = DIST, work = WORK, offline = false, root = EXTENSIONS} = {}) {
  const problems = checkRecipe(id, recipe, {root});
  assert(!problems.length, `${id}:\n  ${problems.join('\n  ')}`);
  const extDir = path.join(root, id);
  const config = loadConfig();
  const tools = fs.existsSync(toolchainFile()) ? loadToolchain() : {};
  // The toolchain is fetched only when a grammar actually has to be compiled.
  let prepared;
  const getTools = async () => {
    assert(tools['cli-version'], 'tools/toolchain.toml is needed to compile a grammar');
    return (prepared ??= await prepareTools(tools, {work, offline}));
  };
  const prepare = sourcePreparer(recipe.sources, {work, offline, extDir});
  const stage = stageDirectory(id, ANY, dist);
  removeUnder(stageRoot(dist), stage);
  fs.mkdirSync(stage, {recursive: true});

  const metadata = [], usedSources = new Set();
  const readQuery = async (reference, source) => {
    const name = reference.source || source;
    usedSources.add(name);
    return fs.readFileSync(inside(await prepare(name), reference.path), 'utf8');
  };
  for (const [name, grammar] of Object.entries(recipe.grammars)) {
    usedSources.add(grammar.source);
    for (const support of grammar.support ?? []) usedSources.add(support.source);
    let source = await prepare(grammar.source);
    const grammarPath = grammar.path || '.';
    if (grammar.generate) {
      const toolchain = await getTools();
      const generated = path.join(work, 'generated', `${id}-${name}`);
      removeUnder(work, generated);
      fs.cpSync(source, generated, {recursive: true, dereference: true});
      source = generated;
      const cwd = inside(source, grammarPath);
      run(toolchain.executable, ['generate', '--abi', String(tools['generate-abi']), ...(fs.existsSync(path.join(cwd, 'src/grammar.json')) ? ['src/grammar.json'] : [])], {cwd, env: toolchain.env});
    }
    const output = path.join(stage, `tree-sitter-${name}.wasm`);
    if (!grammar.prebuilt) assert(fs.existsSync(path.join(inside(source, grammarPath), 'src/parser.c')), `Missing parser.c for ${name}; enable generate`);
    const key = moduleFingerprint(recipe, tools, name, extDir);
    const cache = path.join(work, 'modules', `${key}.wasm`), receipt = `${cache}.sha256`;
    if (fs.existsSync(cache) && fs.existsSync(receipt) && fileHash(cache) === fs.readFileSync(receipt, 'utf8').trim()) fs.copyFileSync(cache, output);
    else {
      if (grammar.prebuilt) fs.copyFileSync(inside(source, relative(grammar.prebuilt)), output);
      else if (grammar.optimize === false) {
        const toolchain = await getTools();
        // The same WASI shared-module ABI as the CLI, without the optional Binaryen pass.
        const cwd = path.join(inside(source, grammarPath), 'src');
        const clang = path.join(toolchain.wasi, 'bin', process.platform === 'win32' ? 'clang.exe' : 'clang');
        const scanner = fs.existsSync(path.join(cwd, 'scanner.c')) ? ['scanner.c'] : [];
        const support = await Promise.all((grammar.support || []).map(async reference => inside(await prepare(reference.source), reference.path)));
        run(clang, ['--target=wasm32-wasip1', '-o', output, '-fPIC', '-shared', '--no-wasm-opt', '-Os',
          `-Wl,--export=tree_sitter_${name}`, '-Wl,--allow-undefined', '-Wl,--no-entry', '-nostdlib', '-fno-exceptions', '-fvisibility=hidden',
          '-I', '.', 'parser.c', ...scanner, ...support], {cwd, env: toolchain.env});
      } else {
        const toolchain = await getTools();
        run(toolchain.executable, ['build', '--wasm', '--output', output, inside(source, grammarPath)], {env: toolchain.env});
      }
      fs.mkdirSync(path.dirname(cache), {recursive: true});
      fs.copyFileSync(output, cache);
      writeAtomic(receipt, `${fileHash(cache)}\n`);
    }
    const entry = {name, path: '.', 'file-types': grammar['file-types']};
    for (const kind of ['highlights', 'injections']) {
      const references = grammar[kind] || [];
      if (!references.length) continue;
      const text = rewriteCaptures((await Promise.all(references.map(reference => readQuery(reference, grammar.source)))).join('\n'), kind === 'highlights' ? grammar['capture-map'] : {});
      const file = `queries/${name}/${kind}.scm`;
      fs.mkdirSync(path.dirname(path.join(stage, file)), {recursive: true});
      fs.writeFileSync(path.join(stage, file), text);
      entry[kind] = file;
    }
    metadata.push(entry);
  }

  // Licenses of every source that went in, with the ones those depend on.
  const licensed = new Set();
  const addSource = name => {
    if (licensed.has(name)) return;
    licensed.add(name);
    for (const dependency of recipe.sources[name].dependencies ?? []) addSource(dependency.source);
  };
  usedSources.forEach(addSource);
  const licenseTexts = [];
  for (const name of [...licensed].sort()) {
    const source = await prepare(name);
    for (const [index, file] of recipe.sources[name].licenses.entries()) {
      const output = path.join(stage, 'licenses', `${name}-${index}-${path.basename(file)}`);
      fs.mkdirSync(path.dirname(output), {recursive: true});
      fs.copyFileSync(inside(source, file), output);
      licenseTexts.push(fs.readFileSync(output, 'utf8'));
    }
  }
  fs.writeFileSync(path.join(stage, 'tree-sitter.json'), `${JSON.stringify({grammars: metadata}, null, 2)}\n`);
  for (const patch of recipe.patches ?? []) applyQueryPatch(stage, inside(extDir, patch));

  const license = licenseExpression(licenseTexts);
  const manifest = languageManifest(id, recipe, {repository: config.repository, license, grammars: Object.keys(recipe.grammars)});
  fs.writeFileSync(path.join(stage, 'manifest.json'), `${JSON.stringify(manifest, null, 2)}\n`);
  const provenance = [...licensed].sort().map(name => {
    const spec = recipe.sources[name];
    return `- ${name}: ${spec.repository ? `${spec.repository} @ ${spec.revision}` : `loom-extensions / ${id} / ${spec.directory}`}\n`;
  }).join('');
  fs.writeFileSync(path.join(stage, 'PROVENANCE.md'),
    `# ${recipe.extension.name}\n\nBuilt by loom-extensions with the Tree-sitter CLI ${tools['cli-version']}.\n\n${provenance}` +
    (recipe.patches?.length ? `\nApplied patches: ${recipe.patches.join(', ')}\n` : '') +
    Object.entries(recipe.grammars).filter(([, g]) => g['capture-map']).map(([name, g]) => `\nCapture adaptations (${name}): ${JSON.stringify(g['capture-map'])}\n`).join('') +
    Object.entries(recipe.grammars).filter(([, g]) => g.prebuilt).map(([name, g]) => `\nPrebuilt module (${name}): ${g.source}/${g.prebuilt}, pinned by its source archive.\n`).join(''));
  for (const [file, content] of Object.entries(documentFiles(id, recipe, {license, badge: badge(recipe.extension.name)}))) {
    fs.writeFileSync(path.join(stage, file), content);
  }
  const checked = checkExtension(stage, {built: true});
  assert(!checked.problems.length, `${id}:\n  ${checked.problems.join('\n  ')}`);
  return [ANY];
}
