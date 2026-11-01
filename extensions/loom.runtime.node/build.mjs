// Builds loom.runtime.node into dist/loom.runtime.node-<version>-<platform>.loomext
// for every platform in pins.json, or for the ones named: node build.mjs [platform…]
//   npm ci && node build.mjs
// Each package is the files named below plus the official Node.js release's program
// and license, checked against the hash pinned in pins.json. This project depends on
// nothing outside itself except what package.json names.
import fs from 'node:fs';
import path from 'node:path';
import {createHash} from 'node:crypto';
import {Readable} from 'node:stream';
import {pipeline} from 'node:stream/promises';
import AdmZip from 'adm-zip';
import * as tar from 'tar';
import xz from 'xz-decompress';

const {XzReadableStream} = xz;
const FILES = ['manifest.json', 'README.md', 'CHANGELOG.md', 'icon.svg'];

const here = import.meta.dirname;
const pins = JSON.parse(fs.readFileSync(path.join(here, 'pins.json'), 'utf8'));
const manifest = JSON.parse(fs.readFileSync(path.join(here, 'manifest.json'), 'utf8'));
const work = path.join(here, '.work'), dist = path.join(here, 'dist');
const platforms = process.argv.length > 2 ? process.argv.slice(2) : Object.keys(pins.platforms);
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');

async function download({url, sha256: expected}, file) {
  if (fs.existsSync(file) && sha256(fs.readFileSync(file)) === expected) return;
  if (new URL(url).protocol !== 'https:') throw new Error(`only https downloads are allowed: ${url}`);
  const response = await fetch(url, {signal: AbortSignal.timeout(600000)});
  if (!response.ok) throw new Error(`${url}: HTTP ${response.status}`);
  const bytes = Buffer.from(await response.arrayBuffer());
  if (sha256(bytes) !== expected) throw new Error(`${url}: the download does not match its pinned SHA-256`);
  fs.mkdirSync(path.dirname(file), {recursive: true});
  fs.writeFileSync(file + '.partial', bytes);
  fs.renameSync(file + '.partial', file);
}

for (const platform of platforms) {
  const pin = pins.platforms[platform];
  if (!pin) throw new Error(`no pinned Node.js release for ${platform}`);
  const archive = path.join(work, `${pin.sha256}.${pin.format}`);
  await download(pin, archive);

  // Only the program and the license are taken out of the archive.
  const top = path.basename(new URL(pin.url).pathname).replace(/\.(zip|tar\.gz|tar\.xz)$/, '');
  const wanted = [pin.program, 'LICENSE'];
  const unpacked = path.join(work, 'unpacked', platform);
  fs.rmSync(unpacked, {recursive: true, force: true});
  fs.mkdirSync(unpacked, {recursive: true});
  if (pin.format === 'zip') {
    const zip = new AdmZip(archive);
    for (const name of wanted) {
      const entry = zip.getEntry(`${top}/${name}`);
      if (!entry) throw new Error(`${top}/${name} is not in the archive`);
      fs.mkdirSync(path.dirname(path.join(unpacked, name)), {recursive: true});
      fs.writeFileSync(path.join(unpacked, name), entry.getData());
    }
  } else {
    const source = fs.createReadStream(archive);
    const input = pin.format === 'tar.xz' ? Readable.fromWeb(new XzReadableStream(Readable.toWeb(source))) : source;
    await pipeline(input, tar.x({cwd: unpacked, strip: 1, filter: name => wanted.some(item => name === `${top}/${item}`)}));
  }

  // The package folder, named after the extension.
  const folder = path.join(dist, 'package', platform, manifest.id);
  fs.rmSync(path.join(dist, 'package', platform), {recursive: true, force: true});
  fs.mkdirSync(folder, {recursive: true});
  for (const file of FILES) fs.copyFileSync(path.join(here, file), path.join(folder, file));
  const program = path.basename(pin.program);
  fs.copyFileSync(path.join(unpacked, ...pin.program.split('/')), path.join(folder, program));
  fs.copyFileSync(path.join(unpacked, 'LICENSE'), path.join(folder, 'LICENSE'));
  fs.writeFileSync(path.join(folder, 'manifest.json'), JSON.stringify({...manifest, executables: [program]}, null, 2) + '\n');
  fs.writeFileSync(path.join(folder, 'PROVENANCE.md'), `# Node.js ${pins.version}\n\nRepackaged unchanged from ${pin.url}\nSHA-256 ${pin.sha256}\n`);

  // A .loomext is a gzip tar whose one root folder is the extension id.
  const output = path.join(dist, `${manifest.id}-${manifest.version}-${platform}.loomext`);
  const names = fs.readdirSync(folder, {recursive: true, withFileTypes: true}).filter(entry => entry.isFile())
    .map(entry => path.relative(folder, path.join(entry.parentPath, entry.name)).split(path.sep).join('/')).sort();
  await tar.c({file: output, cwd: path.dirname(folder), gzip: {level: 9}, portable: true, mtime: new Date(0), noDirRecurse: true},
    [manifest.id, ...names.map(name => `${manifest.id}/${name}`)]);
  console.log(`Wrote ${path.relative(here, output)}`);
}
