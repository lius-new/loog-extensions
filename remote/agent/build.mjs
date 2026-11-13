// Target artifacts are owned and bundled by the remote packages, never by Loog.
// One static Linux agent per CPU architecture; the client picks the one the target reports.
import fs from 'node:fs';
import path from 'node:path';
import {execFileSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {prepareSdk} from '../../scripts/lib/sdk.mjs';

export const AGENT_TARGETS = [
  {machine: 'x86_64', triple: 'x86_64-unknown-linux-musl'},
  {machine: 'aarch64', triple: 'aarch64-unknown-linux-musl'},
];

export function buildAgents() {
  prepareSdk(path.resolve(import.meta.dirname, '../..'), process.env.LOOG_SOURCE);
  const directory = import.meta.dirname;
  const targetDir = process.env.CARGO_TARGET_DIR ? path.resolve(directory, process.env.CARGO_TARGET_DIR) : path.join(directory, 'target');
  const bundle = path.join(directory, 'target', 'bundle');
  fs.rmSync(bundle, {recursive: true, force: true});
  fs.mkdirSync(bundle, {recursive: true});
  const agents = [];
  for (const {machine, triple} of AGENT_TARGETS) {
    // rust-lld links every architecture, so no cross C toolchain is needed.
    const variable = `CARGO_TARGET_${triple.toUpperCase().replaceAll('-', '_')}_LINKER`;
    execFileSync('cargo', ['build', '--manifest-path', path.join(directory, 'Cargo.toml'), '--locked', '--release', '--target', triple],
      {cwd: directory, env: {...process.env, [variable]: 'rust-lld'}, stdio: 'inherit', windowsHide: true});
    const binary = fs.readFileSync(path.join(targetDir, triple, 'release/loog-remote-agent'));
    if (binary.length > 32 * 1024 * 1024) throw new Error('The agent exceeds its deployment budget');
    fs.writeFileSync(path.join(bundle, machine), binary);
    agents.push({machine, triple, binary, sha256: createHash('sha256').update(binary).digest('hex')});
  }
  // build.rs reads this to embed the digests next to the binaries.
  fs.writeFileSync(path.join(bundle, 'agents.txt'), agents.map(agent => `${agent.machine} ${agent.binary.length} ${agent.sha256}\n`).join(''));
  return {agents, directory};
}
