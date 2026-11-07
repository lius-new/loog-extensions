#!/usr/bin/env node
// The loog-extensions command line. Each command is a module in ./commands.
import {main} from './lib/cli.mjs';

const COMMANDS = ['check', 'dev', 'try', 'lock', 'keygen', 'reindex', 'release'];

main(async () => {
  const [name, ...rest] = process.argv.slice(2);
  if (!name || name === '--help' || name === 'help') {
    console.log('Usage: loog-ext <command> [options]\n');
    for (const command of COMMANDS) console.log('  ' + (await import(`./commands/${command}.mjs`)).usage);
    return;
  }
  if (!COMMANDS.includes(name)) throw new Error(`Unknown command: ${name}. Try: ${COMMANDS.join(', ')}`);
  await (await import(`./commands/${name}.mjs`)).run(rest);
});
