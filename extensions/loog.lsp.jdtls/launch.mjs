import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {spawn, spawnSync} from 'node:child_process';

const server = path.join(import.meta.dirname, 'server');
const arm = process.arch === 'arm64';
const configuration = {
  win32: 'config_win',
  linux: arm ? 'config_linux_arm' : 'config_linux',
  darwin: arm ? 'config_mac_arm' : 'config_mac',
}[process.platform];
if (!configuration) throw new Error(`Unsupported Java language server platform: ${process.platform}`);

const plugins = path.join(server, 'plugins');
const launchers = fs.readdirSync(plugins).filter(file => /^org\.eclipse\.equinox\.launcher_.*\.jar$/.test(file));
if (launchers.length !== 1) throw new Error('Expected one Eclipse Equinox launcher');
const javaName = process.platform === 'win32' ? 'java.exe' : 'java';
const java = process.env.JAVA_HOME ? path.join(process.env.JAVA_HOME, 'bin', javaName) : javaName;
const check = spawnSync(java, ['-version'], {encoding: 'utf8', timeout: 5000, windowsHide: true});
const banner = (check.stdout ?? '') + (check.stderr ?? '');
const version = banner.match(/version\s+"(\d+)/)?.[1];
if (check.status !== 0 || !version || Number(version) < 25) {
  console.error('Eclipse JDT LS 1.63.0 needs JDK 25 or newer. Set JAVA_HOME or update PATH.');
  process.exit(1);
}
const session = fs.mkdtempSync(path.join(os.tmpdir(), 'loog-jdtls-'));
const child = spawn(java, [
  '-Declipse.application=org.eclipse.jdt.ls.core.id1',
  '-Declipse.product=org.eclipse.jdt.ls.core.product',
  '-Dosgi.bundles.defaultStartLevel=4',
  '-Dosgi.checkConfiguration=true',
  `-Dosgi.sharedConfiguration.area=${path.join(server, configuration)}`,
  '-Dosgi.sharedConfiguration.area.readOnly=true',
  '-Dosgi.configuration.cascaded=true',
  '--add-modules=ALL-SYSTEM',
  '--add-opens=java.base/java.util=ALL-UNNAMED',
  '--add-opens=java.base/java.lang=ALL-UNNAMED',
  '-jar', path.join(plugins, launchers[0]),
  '-configuration', path.join(session, 'config'),
  '-data', path.join(session, 'workspace'),
], {stdio: ['pipe', 'inherit', 'inherit'], windowsHide: true});

if (child.stdin) process.stdin.pipe(child.stdin);
child.stdin?.on('error', error => {
  if (error.code !== 'EPIPE' && error.code !== 'ERR_STREAM_DESTROYED') console.error(error.message);
});
child.once('error', error => console.error(`Could not launch JDT LS with Java: ${error.message}`));
process.stdin.once('end', () => child.kill());
process.once('SIGINT', () => child.kill());
process.once('SIGTERM', () => child.kill());
child.once('close', code => {
  process.stdin.destroy();
  try {
    fs.rmSync(session, {recursive: true, force: true});
  } catch (error) {
    console.error(`Could not remove the JDT LS session directory: ${error.message}`);
  }
  process.exitCode = code === 0 ? 0 : 1;
});
