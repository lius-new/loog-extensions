// Take in finished packages and publish them: `intake` checks the packages and records
// what is to be published, `publish` uploads the releases and updates the signed index.
import fs from 'node:fs';
import path from 'node:path';
import {ROOT, assert, readJson, writeAtomic} from '../lib/util.mjs';
import {loadConfig} from '../lib/config.mjs';
import {findValidator} from '../lib/validator.mjs';
import {revision, readRegistry, finishRelease, releaseDirectory, checkSigningKey, fingerprint} from '../lib/release.mjs';
import {intake, packageFiles} from '../lib/intake.mjs';
import {run as lock} from './lock.mjs';

export const usage = 'release <preflight|intake <packages…> [--dry-run]|publish>    Check the signing key, take in finished packages, publish releases and the signed index';
const directory = releaseDirectory();
const planFile = path.join(directory, 'plan.json'), outcomesFile = path.join(directory, 'prepared.json');

function summary(text) {
  console.log(text);
  if (process.env.GITHUB_STEP_SUMMARY) fs.appendFileSync(process.env.GITHUB_STEP_SUMMARY, text + '\n');
}

async function takeIn(argv, config) {
  const dryRun = argv.includes('--dry-run');
  const files = packageFiles(argv.filter(item => item !== '--dry-run'));
  const validator = findValidator();
  assert(validator, 'Intake requires loom-ext-validate');
  const current = dryRun ? null : await readRegistry(config);
  const results = await intake(files, {validator, index: current?.index ?? null, repository: dryRun ? null : config.repository});
  for (const result of results) {
    summary(result.error ? `Rejected ${result.id}: ${result.error}` : result.skipped ? `Skipped ${result.id}: ${result.skipped}`
      : `Accepted ${result.id} ${result.version} (${result.records.map(record => record.platform).join(', ')})`);
  }
  if (dryRun) { assert(!results.some(result => result.error), 'Some packages were rejected'); return; }
  const taken = results.filter(result => !result.skipped);
  const plan = {schema: 1, revision: revision(), sequence: current.index.sequence, repair: current.recovered,
    pending: taken.map(result => ({id: result.id, version: result.version ?? '', fingerprint: result.source?.fingerprint ?? fingerprint(ROOT, result.id),
      platforms: (result.records ?? []).map(record => record.platform).sort()})),
    skipped: results.filter(result => result.skipped).map(result => result.id)};
  writeAtomic(planFile, JSON.stringify(plan, null, 2) + '\n');
  writeAtomic(outcomesFile, JSON.stringify(taken, null, 2) + '\n');
}

export async function run(argv) {
  assert(argv[0] === 'intake' ? argv.length > 1 : argv.length === 1 && ['preflight', 'publish'].includes(argv[0]), usage);
  const config = loadConfig();
  if (argv[0] === 'intake') { await takeIn(argv.slice(1), config); return; }
  if (argv[0] === 'preflight') { checkSigningKey(config, process.env.LOOM_INDEX_KEY); return; }
  const plan = readJson(planFile);
  assert(plan.schema === 1 && plan.revision === revision(), 'The intake is not for this checkout');
  if (!plan.pending.length && !plan.repair) { summary('Nothing to publish; nothing was changed.'); return; }
  const {execFileSync} = await import('node:child_process');
  assert(!execFileSync('git', ['status', '--porcelain', '--untracked-files=normal'], {cwd: ROOT, encoding: 'utf8'}).trim(), 'Commit the working tree before publishing; release tags must describe the tested source');
  const report = await finishRelease(plan, readJson(outcomesFile), {config, privateKey: process.env.LOOM_INDEX_KEY, sourceRevision: revision()});
  summary(`Indexed: ${report.indexed.length}; failures: ${report.failures.length}; sequence: ${report.sequence}.\n\n` + report.failures.map(e => `- ${e.id}: ${e.error}`).join('\n'));
  assert(!report.failures.length, 'Some extensions were not delivered; see the release report. Rerun to complete them.');
  await lock([]);
}
