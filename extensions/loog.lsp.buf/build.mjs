// Builds loog.lsp.buf into dist/loog.lsp.buf-<version>-<platform>.loogext for every platform in
// pins.json, or for the ones named: node build.mjs [platform…]
//   npm ci && node build.mjs
// Each package is the upstream release archive repackaged unchanged: what pins.json
// says to include, checked against the pinned hash, plus the Loog WASM client.
import fs from 'node:fs';
import path from 'node:path';
import {createHash} from 'node:crypto';
import * as tar from 'tar';
import { buildClient, clientNotices } from '../../language-clients/rust/build.mjs';

const FILES = ['manifest.json', 'README.md', 'CHANGELOG.md', 'icon.svg'];

const here = import.meta.dirname;
const component = buildClient(here);
const pins = JSON.parse(fs.readFileSync(path.join(here, 'pins.json'), 'utf8'));
const manifest = JSON.parse(fs.readFileSync(path.join(here, 'manifest.json'), 'utf8'));
const work = path.join(here, '.work'), dist = path.join(here, 'dist');
const platforms = process.argv.length > 2 ? process.argv.slice(2) : Object.keys(pins.platforms);
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');

async function download({url, sha256: expected}, file) {
  if (fs.existsSync(file) && sha256(fs.readFileSync(file)) === expected) return file;
  if (new URL(url).protocol !== 'https:') throw new Error(`only https downloads are allowed: ${url}`);
  const response = await fetch(url, {signal: AbortSignal.timeout(1800000)});
  if (!response.ok) throw new Error(`${url}: HTTP ${response.status}`);
  const bytes = Buffer.from(await response.arrayBuffer());
  if (sha256(bytes) !== expected) throw new Error(`${url}: the download does not match its pinned SHA-256`);
  fs.mkdirSync(path.dirname(file), {recursive: true});
  fs.writeFileSync(file + '.partial', bytes);
  fs.renameSync(file + '.partial', file);
  return file;
}

for (const platform of platforms) {
  const pin = pins.platforms[platform];
  if (!pin) throw new Error(`no pinned release for ${platform}`);
  const archive = await download(pin, path.join(work, 'downloads', pin.sha256));

  // The package folder, named after the extension.
  const folder = path.join(dist, 'package', platform, manifest.id);
  fs.rmSync(path.join(dist, 'package', platform), {recursive: true, force: true});
  fs.mkdirSync(folder, {recursive: true});
  fs.copyFileSync(archive, path.join(folder, pin.executables[0]));
  for (const name of pin.executables) {
    const file = path.join(folder, ...name.split('/'));
    if (!fs.existsSync(file) || !fs.statSync(file).isFile()) throw new Error(`${platform}: the archive has no ${name}`);
    fs.chmodSync(file, 0o755);
  }
  for (const [name, license] of Object.entries(pins.licenses ?? {})) {
    fs.mkdirSync(path.join(folder, 'licenses'), {recursive: true});
    fs.copyFileSync(await download(license, path.join(work, 'downloads', license.sha256)), path.join(folder, 'licenses', name));
  }
  for (const file of FILES) fs.copyFileSync(path.join(here, file), path.join(folder, file));
  fs.cpSync(path.join(here, 'languages'), path.join(folder, 'languages'), {recursive: true});
  fs.copyFileSync(component, path.join(folder, 'extension.wasm'));
  clientNotices(folder, here);
  fs.writeFileSync(path.join(folder, 'manifest.json'), JSON.stringify({...manifest, executables: pin.executables}, null, 2) + '\n');
  fs.writeFileSync(path.join(folder, 'PROVENANCE.md'), `# ${manifest.name} ${pins.version}\n\nRepackaged from the upstream release.\n\nPlatform: ${platform}\nLicense: ${manifest.license}\nUpstream: ${pin.url}\nSHA-256: ${pin.sha256}\n`);

  // A .loogext is a gzip tar whose one root folder is the extension id.
  const output = path.join(dist, `${manifest.id}-${manifest.version}-${platform}.loogext`);
  const names = fs.readdirSync(folder, {recursive: true, withFileTypes: true}).filter(entry => entry.isFile())
    .map(entry => path.relative(folder, path.join(entry.parentPath, entry.name)).split(path.sep).join('/')).sort();
  await tar.c({file: output, cwd: path.dirname(folder), gzip: {level: 9}, portable: true, mtime: new Date(0), noDirRecurse: true},
    [manifest.id, ...names.map(name => `${manifest.id}/${name}`)]);
  console.log(`Wrote ${path.relative(here, output)}`);
}
