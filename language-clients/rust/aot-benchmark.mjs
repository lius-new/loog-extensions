// No builds run during this script. Every measurement starts a fresh process.
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import {spawnSync,execFileSync} from 'node:child_process';
import {createHash} from 'node:crypto';
const usage='node language-clients/rust/aot-benchmark.mjs --loom <checkout> --host <release-test-executable> --baseline <extensions-root> --candidate <extensions-root> --out <empty-directory> [--language rust|javascript|typescript|go] [--runs 3] [--cost <component_cost-executable>] [--test-authority <benchmark-public-key-file>]';
const argv=process.argv.slice(2);
if(argv.includes('--help')){console.log(usage);process.exit(0);}
const options={};
for(let at=0;at<argv.length;at+=2){const key=argv[at];if(!['--loom','--host','--baseline','--candidate','--out','--runs','--cost','--language','--test-authority'].includes(key)||!argv[at+1])throw new Error(usage);options[key.slice(2)]=argv[at+1];}
for(const key of ['host','baseline','candidate','out'])if(!options[key])throw new Error('Missing --'+key+'; '+usage);
const loom=path.resolve(options.loom??process.env.LOOM_SOURCE??path.join(import.meta.dirname,'../../../..'));
const extensions=path.resolve(import.meta.dirname,'../..');
const executable=path.resolve(options.host);
const cost=path.resolve(options.cost??path.join(loom,'target/release/examples/component_cost'+(process.platform==='win32'?'.exe':'')));
const destination=path.resolve(options.out);
if(fs.existsSync(destination)&&fs.readdirSync(destination).length)throw new Error('--out must be empty so cold-cache measurements cannot reuse old artifacts');
fs.mkdirSync(destination,{recursive:true});
const repeats=Number(options.runs??3);
if(!Number.isInteger(repeats)||repeats<1||repeats>10)throw new Error('--runs must be between 1 and 10');
const digest=file=>createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const packages={wasm:path.resolve(options.baseline),aot:path.resolve(options.candidate)};
const publicKeyFile=options['test-authority']?path.resolve(options['test-authority']):null;
const publicKey=publicKeyFile?fs.readFileSync(publicKeyFile,'utf8').trim():null;
const language=options.language??'rust';
const ids={rust:'loom.lsp.rust-analyzer',javascript:'loom.lsp.typescript-language-server',typescript:'loom.lsp.typescript-language-server',go:'loom.lsp.gopls'};
const id=ids[language];if(!id)throw new Error('Unsupported --language');
const versions=Object.fromEntries(['wasm','aot'].map(route=>[route,JSON.parse(fs.readFileSync(path.join(packages[route],'extensions.json'),'utf8')).extensions[id].version]));
const components=Object.fromEntries(['wasm','aot'].map(route=>[route,path.join(packages[route],id,versions[route],'extension.wasm')]));
const server=route=>path.join(packages[route],id,versions[route],(language==='rust'?'rust-analyzer':'gopls')+(process.platform==='win32'?'.exe':''));
function treeDigest(root){
  const hash=createHash('sha256');
  function walk(folder){for(const name of fs.readdirSync(folder).sort()){
    const file=path.join(folder,name);if(fs.statSync(file).isDirectory())walk(file);
    else {hash.update(path.relative(root,file).split(path.sep).join('/'));hash.update('\0');hash.update(fs.readFileSync(file));hash.update('\0');}
  }}walk(root);return hash.digest('hex');
}
const serverDigest=route=>language==='javascript'||language==='typescript'
 ?treeDigest(path.join(packages[route],id,versions[route],'node_modules')):digest(server(route));
if(digest(components.wasm)!==digest(components.aot))throw new Error('WASM inputs differ; this is an AOT-only comparison');
if(serverDigest('wasm')!==serverDigest('aot'))throw new Error('Server payloads differ');
const runtime=route=>{const index=JSON.parse(fs.readFileSync(path.join(packages[route],'extensions.json'),'utf8'));const version=index.extensions['loom.runtime.node']?.version;return version?path.join(packages[route],'loom.runtime.node',version,'node'+(process.platform==='win32'?'.exe':'')):null;};
if((language==='javascript'||language==='typescript')&&digest(runtime('wasm'))!==digest(runtime('aot')))throw new Error('Node runtimes differ');
function run(command,args,env,label){
  const result=spawnSync(command,args,{cwd:loom,env:{...process.env,...env},encoding:'utf8',windowsHide:true,maxBuffer:16*1024*1024,timeout:180000});
  fs.writeFileSync(path.join(destination,label+'.log'),(result.stdout??'')+(result.stderr??''));
  if(result.status!==0)throw new Error(`${label} failed (${result.status}): ${result.error??''}\n${result.stderr??''}\n${result.stdout??''}`);
  return result.stdout;
}
function native(route,bytes,label,cache,checks=false){
  const env={LOOM_MIGRATION_LANGUAGE:language,LOOM_MIGRATION_BYTES:String(bytes),LOOM_MIGRATION_PACKAGES:packages[route],LOOM_MIGRATION_CACHE:cache};
  // Empty values count as present for var_os, so remove unrelated inherited switches.
  const inherited=process.env;
  delete inherited.LOOM_MIGRATION_COMPARE;delete inherited.LOOM_MIGRATION_FULL_CHECKS;delete inherited.LOOM_MIGRATION_VARIANTS;delete inherited.LOOM_MIGRATION_TRACE;delete inherited.LOOM_MIGRATION_REQUIRE_AOT;delete inherited.LOOM_MIGRATION_AOT_PUBLIC_KEY;
  if(publicKey)env.LOOM_MIGRATION_AOT_PUBLIC_KEY=publicKey;
  if(route==='aot')env.LOOM_MIGRATION_REQUIRE_AOT='1';
  if(checks)env.LOOM_MIGRATION_FULL_CHECKS='1';
  run(executable,['--ignored','--exact','app::extensions::tests::official_language_components_reach_native_consumers_without_host_servers','--nocapture','--test-threads=1'],env,label);
  const root=path.join(loom,'target/language-migration/projects',bytes===0?language:`${language}-${bytes}`);
  const report=JSON.parse(fs.readFileSync(path.join(root,'native-observed.json'),'utf8'));
  if(report.code_source!==(route==='aot'?'published-aot':label.includes('cold')?'compiled':'local-cache'))throw new Error(label+': unexpected code source '+report.code_source);
  for(const item of Object.values(report.small_document)){if(item.count!==100||item.samples_ms.length!==100)throw new Error(`${label}: incomplete cohort`);}
  fs.writeFileSync(path.join(destination,label+'.json'),JSON.stringify({route,cache_mode:label.includes('cold')?'cold':'warm',...report},null,2)+'\n');
  console.log(`${label}: initialized=${report.initialized_ms.toFixed(1)} ms, usable=${report.first_usable_ms.toFixed(1)} ms; completion p95=${report.small_document.Completion.p95_ms.toFixed(3)} ms`);
}
const metadata={aot_profile:'loom-wasmtime-50.0.1-portable-v1',benchmark_authority:publicKey?'temporary test key; not a production trust override':'official',language,plugin:id,versions,started_utc:new Date().toISOString(),timezone:'Asia/Shanghai',os:`${os.platform()} ${os.release()} ${os.arch()}`,cpu:os.cpus()[0].model,total_memory_bytes:os.totalmem(),
  rust:execFileSync('rustc',['--version'],{encoding:'utf8'}).trim(),node:process.version,
  loom_commit:execFileSync('git',['-c','safe.directory='+loom,'rev-parse','HEAD'],{cwd:loom,encoding:'utf8'}).trim(),
  extensions_commit:execFileSync('git',['-c','safe.directory='+extensions,'rev-parse','HEAD'],{cwd:extensions,encoding:'utf8'}).trim(),
  component_sha256:Object.fromEntries(['wasm','aot'].map(route=>[route,digest(components[route])])),
  component_bytes:Object.fromEntries(['wasm','aot'].map(route=>[route,fs.statSync(components[route]).size])),
  server_sha256:serverDigest('wasm'),
  server_version:language==='rust'?execFileSync(server('wasm'),['--version'],{encoding:'utf8'}).trim():language==='go'?execFileSync(server('wasm'),['version'],{encoding:'utf8'}).trim():JSON.parse(fs.readFileSync(path.join(packages.wasm,id,versions.wasm,'node_modules/typescript-language-server/package.json'),'utf8')).version,
  node_runtime_sha256:runtime('wasm')?digest(runtime('wasm')):null,
  executable_sha256:digest(executable),cost_executable_sha256:digest(cost),
  methodology:{native_cold_repeats:repeats,native_repeats:repeats,samples_per_operation_per_repeat:100,warmups_per_operation:10,readiness:'10 consecutive usable completion/hover/definition probes and at least 2 seconds; first usable time recorded separately',sizes:[0,1048576,8388608],memory:'Loom process resident/private bytes, excluding separately running language servers and their child processes; TS server still uses Node; pre-activation snapshots retained',component_repeats:repeats,builds_during_measurement:false},
};
fs.writeFileSync(path.join(destination,'metadata.json'),JSON.stringify(metadata,null,2)+'\n');
// Separate cold end-to-end measurements also exercise the full negotiated surface.
for(let repeat=1;repeat<=repeats;repeat++)for(const route of (repeat%2?['wasm','aot']:['aot','wasm']))
  native(route,0,`native-${route}-cold-${repeat}`,path.join(destination,`cache-native-${route}-cold-${repeat}`),repeat===1);
for(const bytes of [0,1048576,8388608]){
  for(let repeat=0;repeat<repeats;repeat++){
    const order=repeat%2===0?['wasm','aot']:['aot','wasm'];
    for(const route of order)native(route,bytes,`native-${route}-${bytes}-${repeat+1}`,path.join(destination,`cache-native-${route}-cold-1`));
  }
}
for(let repeat=0;repeat<repeats;repeat++){
  for(const route of (repeat%2===0?['wasm','aot']:['aot','wasm'])){
    const cache=path.join(destination,`cache-component-${route}-${repeat+1}`);
    for(const mode of ['cold','warm']){
      const label=`component-${route}-${mode}-${repeat+1}`;
      const result=JSON.parse(run(cost,[components[route],cache,path.dirname(components[route]),...(publicKeyFile?[publicKeyFile]:[])],{},label).trim());
      if(result.code_source!==(route==='aot'?'published-aot':mode==='warm'?'local-cache':'compiled'))throw new Error(label+': incorrect code source');
      fs.writeFileSync(path.join(destination,label+'.json'),JSON.stringify({route,mode,...result},null,2)+'\n');
      console.log(`${label}: compile/load=${result.compile_or_load_ms.toFixed(2)} ms, instantiate=${result.instantiate_ms.toFixed(2)} ms`);
    }
  }
}
metadata.finished_utc=new Date().toISOString();fs.writeFileSync(path.join(destination,'metadata.json'),JSON.stringify(metadata,null,2)+'\n');
console.log('All cohorts completed without dropped requests.');
