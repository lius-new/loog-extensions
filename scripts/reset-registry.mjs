// Explicit maintenance operation, invoked only by a confirmed manual Release run.
import {loadConfig} from './lib/config.mjs';
import {resetRegistry} from './lib/reset-registry.mjs';
const report = await resetRegistry({config: loadConfig(), privateKey: process.env.LOOM_INDEX_KEY,
  confirmed: process.argv.length === 3 && process.argv[2] === '--confirm'});
console.log('Reset registry to sequence ' + report.sequence + '; deleted ' + report.deleted.length + ' extension releases and tags.');
