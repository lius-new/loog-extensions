// The pinned Loog source commit that builds and verification run against.
import path from 'node:path';
import {assert, ROOT, readToml} from './util.mjs';

export function readLoogSource() {
  const source = readToml(path.join(ROOT, 'tools/loog-source.toml'));
  assert(source.schema === 1 && /^[\w.-]+\/[\w.-]+$/.test(source.repository) && /^[a-f0-9]{40}$/.test(source.revision),
    'Pin the Loog repository and complete source commit in tools/loog-source.toml');
  return {repository: source.repository, revision: source.revision};
}
