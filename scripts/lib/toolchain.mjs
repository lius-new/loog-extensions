// The pinned toolchain (tools/toolchain.toml): the Tree-sitter CLI, the WASI SDK and
// Binaryen, downloaded once per platform and checked against their hashes.
import fs from 'node:fs';
import path from 'node:path';
import {gunzipSync} from 'node:zlib';
import {HASH, ROOT, WORK, assert, readToml, removeUnder, run} from './util.mjs';
import {extractArchive, fetchPinned, oneRoot} from './fetch.mjs';

export const toolchainFile = (root = ROOT) => path.join(root, 'tools', 'toolchain.toml');

export function loadToolchain(root = ROOT) {
  const tools = readToml(toolchainFile(root));
  const problems = checkToolchain(tools);
  assert(!problems.length, `tools/toolchain.toml:\n  ${problems.join('\n  ')}`);
  return tools;
}

export function checkToolchain(tools) {
  const problems = [];
  if (tools.schema !== 1) problems.push('schema must be 1');
  if (!/^\d+\.\d+\.\d+$/.test(tools['cli-version'] ?? '')) problems.push('pin an exact tree-sitter CLI version');
  if (![14, 15].includes(tools['generate-abi'])) problems.push('generate-abi must be 14 or 15');
  for (const kind of ['cli', 'wasi', 'binaryen']) {
    const specs = tools[kind] ?? {};
    if (!Object.keys(specs).length) problems.push(`missing ${kind} toolchain`);
    for (const [platform, spec] of Object.entries(specs)) {
      let https = false;
      try { https = new URL(spec.url).protocol === 'https:'; } catch { /* reported below */ }
      if (!https || !HASH.test(spec.sha256 ?? '')) problems.push(`${kind}.${platform}: pin an https URL and its SHA-256`);
      if (!['gzip', 'tar.gz'].includes(spec.format)) problems.push(`${kind}.${platform}: invalid archive format`);
    }
  }
  return problems;
}

/** Download and unpack the toolchain for this machine; returns paths and the environment to build with. */
export async function prepareTools(tools, options = {}) {
  const work = options.work || WORK;
  const platform = `${process.platform}-${process.arch}`;
  const result = {};
  for (const kind of ['cli', 'wasi', 'binaryen']) {
    const spec = tools[kind][platform];
    assert(spec, `No pinned ${kind} toolchain for ${platform}`);
    const base = path.join(work, 'tools', spec.sha256);
    const archive = await fetchPinned(spec.url, spec.sha256, path.join(base, `input.${spec.format}`), options);
    const content = path.join(base, 'content');
    if (!fs.existsSync(path.join(base, 'complete'))) {
      removeUnder(work, content);
      fs.mkdirSync(content, {recursive: true});
      if (spec.format === 'gzip') {
        fs.writeFileSync(path.join(content, process.platform === 'win32' ? 'tree-sitter.exe' : 'tree-sitter'), gunzipSync(fs.readFileSync(archive)));
      } else await extractArchive(archive, content);
      fs.writeFileSync(path.join(base, 'complete'), spec.sha256);
    }
    result[kind] = oneRoot(content);
  }
  result.executable = path.join(result.cli, process.platform === 'win32' ? 'tree-sitter.exe' : 'tree-sitter');
  if (process.platform !== 'win32') fs.chmodSync(result.executable, 0o755);
  const version = run(result.executable, ['--version']).trim().split(/\s+/).slice(0, 2).join(' ');
  assert(version === `tree-sitter ${tools['cli-version']}`, 'Tree-sitter CLI version mismatch');
  result.env = {
    ...process.env, TREE_SITTER_WASI_SDK_PATH: result.wasi, TREE_SITTER_BINARYEN_PATH: result.binaryen,
    TREE_SITTER_DIR: path.join(work, 'cli-config'), TREE_SITTER_LIBDIR: path.join(work, 'cli-cache'), NO_COLOR: '1',
  };
  return result;
}
