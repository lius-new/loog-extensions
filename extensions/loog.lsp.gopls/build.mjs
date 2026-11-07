// Builds loog.lsp.gopls into dist/loog.lsp.gopls-<version>-<platform>.loogext for every platform below,
// or for the ones named: node build.mjs [platform…]
//   npm ci && node build.mjs        (needs Go on the PATH)
// Each package is the pinned Go module compiled for its platform with the pinned Go
// toolchain. Client build sources stay in this repository; LOOG_SOURCE selects
// the general Loog SDK used to compile the extension's WASM.
import fs from 'node:fs';
import path from 'node:path';
import {execFileSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import * as tar from 'tar';
import { buildClient, clientNotices } from '../../language-clients/rust/build.mjs';

const CONFIG = {
  "go": "1.27.0",
  "module": "golang.org/x/tools/gopls",
  "module-version": "v0.24.0",
  "platforms": {
    "windows-x86_64": {
      "goos": "windows",
      "goarch": "amd64",
      "executables": [
        "gopls.exe"
      ]
    },
    "windows-aarch64": {
      "goos": "windows",
      "goarch": "arm64",
      "executables": [
        "gopls.exe"
      ]
    },
    "macos-x86_64": {
      "goos": "darwin",
      "goarch": "amd64",
      "executables": [
        "gopls"
      ]
    },
    "macos-aarch64": {
      "goos": "darwin",
      "goarch": "arm64",
      "executables": [
        "gopls"
      ]
    },
    "linux-x86_64": {
      "goos": "linux",
      "goarch": "amd64",
      "executables": [
        "gopls"
      ]
    },
    "linux-aarch64": {
      "goos": "linux",
      "goarch": "arm64",
      "executables": [
        "gopls"
      ]
    }
  }
};
const FILES = ['manifest.json', 'README.md', 'CHANGELOG.md', 'icon.svg'];

const here = import.meta.dirname;
const component = buildClient(here);
const pins = JSON.parse(fs.readFileSync(path.join(here, 'pins.json'), 'utf8'));
const manifest = JSON.parse(fs.readFileSync(path.join(here, 'manifest.json'), 'utf8'));
const work = path.join(here, '.work'), dist = path.join(here, 'dist');
const platforms = process.argv.length > 2 ? process.argv.slice(2) : Object.keys(CONFIG.platforms);
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
const module = `${CONFIG.module}@${CONFIG['module-version']}`;

async function download({url, sha256: expected}, file) {
  if (fs.existsSync(file) && sha256(fs.readFileSync(file)) === expected) return file;
  if (new URL(url).protocol !== 'https:') throw new Error(`only https downloads are allowed: ${url}`);
  const response = await fetch(url, {signal: AbortSignal.timeout(600000)});
  if (!response.ok) throw new Error(`${url}: HTTP ${response.status}`);
  const bytes = Buffer.from(await response.arrayBuffer());
  if (sha256(bytes) !== expected) throw new Error(`${url}: the download does not match its pinned SHA-256`);
  fs.mkdirSync(path.dirname(file), {recursive: true});
  fs.writeFileSync(file + '.partial', bytes);
  fs.renameSync(file + '.partial', file);
  return file;
}

for (const platform of platforms) {
  const target = CONFIG.platforms[platform];
  if (!target) throw new Error(`${manifest.id} is not built for ${platform}`);
  const gopath = path.join(work, 'gopath', platform);
  const env = {...process.env, GOPATH: gopath, GOMODCACHE: process.env.GOMODCACHE || path.join(work, 'go-cache', 'modules'),
    GOCACHE: process.env.GOCACHE || path.join(work, 'go-cache', 'build'), GOOS: target.goos, GOARCH: target.goarch,
    CGO_ENABLED: '0', GOTOOLCHAIN: `go${CONFIG.go}`, GOENV: 'off', GOFLAGS: '-trimpath', GOWORK: 'off'};
  // The selected Go executable chooses its own compiler and standard library.
  for (const name of ['GOROOT', 'GOTOOLDIR', 'GOBIN']) delete env[name];
  fs.mkdirSync(work, {recursive: true});
  execFileSync('go', ['install', module], {cwd: work, env, stdio: 'inherit'});
  // A cross-compiled binary lands in a per-target folder.
  const found = target.executables.map(name => [path.join(gopath, 'bin', `${target.goos}_${target.goarch}`, name), path.join(gopath, 'bin', name)].find(file => fs.existsSync(file)));
  if (!found.every(Boolean)) throw new Error(`go install did not produce ${target.executables.join(', ')}`);
  const info = JSON.parse(execFileSync('go', ['mod', 'download', '-json', module], {cwd: work, env: {...env, GOOS: undefined, GOARCH: undefined}, encoding: 'utf8'}));

  // The package folder, named after the extension.
  const folder = path.join(dist, 'package', platform, manifest.id);
  fs.rmSync(path.join(dist, 'package', platform), {recursive: true, force: true});
  fs.mkdirSync(folder, {recursive: true});
  for (const file of FILES) fs.copyFileSync(path.join(here, file), path.join(folder, file));
  fs.cpSync(path.join(here, 'languages'), path.join(folder, 'languages'), { recursive: true });
  fs.copyFileSync(component, path.join(folder, 'extension.wasm'));
  clientNotices(folder, here);
  target.executables.forEach((name, index) => { fs.copyFileSync(found[index], path.join(folder, name)); fs.chmodSync(path.join(folder, name), 0o755); });
  for (const [name, license] of Object.entries(pins.licenses ?? {})) {
    fs.mkdirSync(path.join(folder, 'licenses'), {recursive: true});
    fs.copyFileSync(await download(license, path.join(work, 'downloads', license.sha256)), path.join(folder, 'licenses', name));
  }
  fs.writeFileSync(path.join(folder, 'manifest.json'), JSON.stringify({...manifest, executables: target.executables}, null, 2) + '\n');
  fs.writeFileSync(path.join(folder, 'PROVENANCE.md'), `# ${manifest.name} ${CONFIG['module-version']}\n\nCompiled from the pinned Go module.\n\nPlatform: ${platform}\nLicense: ${manifest.license}\nModule: ${module}\nModule sum: ${info.Sum}\nGo: ${CONFIG.go}\nTarget: ${target.goos}/${target.goarch}\n`);

  // A .loogext is a gzip tar whose one root folder is the extension id.
  const output = path.join(dist, `${manifest.id}-${manifest.version}-${platform}.loogext`);
  const names = fs.readdirSync(folder, {recursive: true, withFileTypes: true}).filter(entry => entry.isFile())
    .map(entry => path.relative(folder, path.join(entry.parentPath, entry.name)).split(path.sep).join('/')).sort();
  await tar.c({file: output, cwd: path.dirname(folder), gzip: {level: 9}, portable: true, mtime: new Date(0), noDirRecurse: true},
    [manifest.id, ...names.map(name => `${manifest.id}/${name}`)]);
  console.log(`Wrote ${path.relative(here, output)}`);
}
