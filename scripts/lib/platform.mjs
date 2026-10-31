// Platforms are written `<os>-<arch>`; data-only packages use `any`.
import {assert} from './util.mjs';

export const PLATFORMS = ['windows-x86_64', 'windows-aarch64', 'linux-x86_64', 'linux-aarch64', 'macos-x86_64', 'macos-aarch64'];
export const ANY = 'any';
export const validPlatform = value => value === ANY || PLATFORMS.includes(value);

const OS = {win32: 'windows', linux: 'linux', darwin: 'macos'};
const ARCH = {x64: 'x86_64', arm64: 'aarch64'};

/** The platform this process runs on, the way the application names it. */
export function hostPlatform(os = process.platform, arch = process.arch) {
  assert(OS[os] && ARCH[arch], `Unsupported platform: ${os}-${arch}`);
  return `${OS[os]}-${ARCH[arch]}`;
}

/** Node's `win32-x64` spelling, as the toolchain pins use it. */
export function nodeKey(platform) {
  assert(PLATFORMS.includes(platform), `Not a platform: ${platform}`);
  const [os, arch] = platform.split('-');
  const key = Object.keys(OS).find(name => OS[name] === os);
  const cpu = Object.keys(ARCH).find(name => ARCH[name] === arch);
  return `${key}-${cpu}`;
}

export const artifactName = (id, version, platform) => {
  assert(validPlatform(platform), `Not a platform: ${platform}`);
  return `${id}-${version}-${platform}.loomext`;
};

/** Which of the artifacts of a version this machine should install, if any. */
export function chooseArtifact(artifacts, platform = hostPlatform()) {
  return artifacts.find(artifact => artifact.platform === platform)
    ?? artifacts.find(artifact => artifact.platform === ANY)
    ?? null;
}
