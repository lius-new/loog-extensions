// Linux-built AOT must deserialize and instantiate on each native release host.
// This corpus uses ephemeral test signatures and never enters publication intake.
import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {ROOT,assert,fileHash,removeUnder} from './lib/util.mjs';
import {extractPackage,createPackage} from './lib/archive.mjs';
import {packageFiles,platformOf} from './lib/intake.mjs';
import {generateKeyPair} from './lib/sign.mjs';
import {prepareAot} from './prepare-aot.mjs';
const clients=['loom.lsp.gopls','loom.lsp.rust-analyzer','loom.lsp.typescript-language-server'];
export function officialInputs(paths){return packageFiles(paths).filter(file=>clients.some(id=>path.basename(file).startsWith(id+'-')));}
const output=value=>{if(process.env.GITHUB_OUTPUT)fs.appendFileSync(process.env.GITHUB_OUTPUT,value+'\n');};
export async function emitCorpus(files,destination,{compiler,work=path.join(ROOT,'.work/aot-contract')}={}){
 fs.mkdirSync(work,{recursive:true});fs.mkdirSync(destination,{recursive:true});
 const components=new Map();
 for(const [i,file]of files.entries()){
  const scratch=path.join(work,'read',String(i));removeUnder(work,scratch);
  const dir=await extractPackage(file,scratch),manifest=JSON.parse(fs.readFileSync(path.join(dir,'manifest.json'),'utf8'));
  assert(clients.includes(manifest.id)&&manifest.component==='extension.wasm','Unexpected conformance input');platformOf(file,manifest);
  const digest=fileHash(path.join(dir,'extension.wasm')),before=components.get(manifest.id);
  if(before){assert(before.digest===digest&&before.version===manifest.version,'Platform build inputs disagree');continue;}
  const thin=path.join(work,'thin',manifest.id);fs.mkdirSync(thin,{recursive:true});
  fs.writeFileSync(path.join(thin,'manifest.json'),JSON.stringify({id:manifest.id,version:manifest.version,component:'extension.wasm'})+'\n');
  fs.copyFileSync(path.join(dir,'extension.wasm'),path.join(thin,'extension.wasm'));
  const archive=path.join(work,'inputs',`${manifest.id}-${manifest.version}-any.loomext`);await createPackage(thin,archive,manifest.id);
  components.set(manifest.id,{digest,version:manifest.version,archive});
 }
 const keys=generateKeyPair();
 const ready=path.join(work,'ready');
 if(components.size){
  await prepareAot([...components.values()].map(item=>item.archive),{compiler,privatePem:keys.privatePem,keyId:'benchmark-only',keys:{'benchmark-only':keys.publicRaw},out:ready,work:path.join(work,'compile')});
  for(const [id,item]of components)await extractPackage(path.join(ready,path.basename(item.archive)),destination,id);
 }
 fs.writeFileSync(path.join(destination,'public-key.txt'),keys.publicRaw+'\n');
 fs.writeFileSync(path.join(destination,'corpus.json'),JSON.stringify({schema:1,profile:'loom-wasmtime-50.0.1-portable-v1',plugins:[...components.keys()],production_installable:false},null,2)+'\n');
 output('count='+components.size);console.log(`Conformance corpus: ${components.size} clients, six targets each`);
}
if(process.argv[1]&&path.resolve(process.argv[1])===fileURLToPath(import.meta.url)){
 const [mode,...args]=process.argv.slice(2);
 if(mode==='plan'){output('has-clients='+Boolean(officialInputs(args).length));}
 else if(mode==='emit'){
  const [destination,...inputs]=args;assert(destination&&inputs.length,'expected emit DEST INPUTS...');
  const compiler=process.env.LOOM_AOT_COMPILER??path.join(ROOT,'tools/aot-compiler/target/release/loom-aot-compiler'+(process.platform==='win32'?'.exe':''));
  await emitCorpus(officialInputs(inputs),path.resolve(destination),{compiler});
 }else throw new Error('expected plan INPUTS... or emit DEST INPUTS...');
}
