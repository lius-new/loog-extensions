// Runs only in the trusted publisher, after plugin build jobs have finished.
// The compiler validates WASM; native bytes from plugin build jobs are discarded.
import fs from 'node:fs';
import path from 'node:path';
import {execFileSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {extractPackage,createPackage} from './lib/archive.mjs';
import {packageFiles,platformOf} from './lib/intake.mjs';
import {signIndex,publicKeyOf} from './lib/sign.mjs';
import {readToml,ROOT,assert,removeUnder,fileHash} from './lib/util.mjs';
export const TARGETS={
 'windows-x86_64':'x86_64-pc-windows-msvc','windows-aarch64':'aarch64-pc-windows-msvc',
 'linux-x86_64':'x86_64-unknown-linux-gnu','linux-aarch64':'aarch64-unknown-linux-gnu',
 'macos-x86_64':'x86_64-apple-darwin','macos-aarch64':'aarch64-apple-darwin',
};
const CLIENTS=new Set(['loog.lsp.gopls','loog.lsp.rust-analyzer','loog.lsp.typescript-language-server']);
const DOMAIN=Buffer.from('loog.extension.aot.v1\0');
export async function prepareAot(files,{compiler,privatePem,keyId,keys,out,work,compile}={}){
 assert(publicKeyOf(privatePem)===keys[keyId],'The AOT compiler signing key is not the configured publication authority');
 fs.mkdirSync(out,{recursive:true});fs.mkdirSync(work,{recursive:true});
 const env={...process.env};delete env.LOOG_INDEX_KEY;
 compile??=((wasm,target,code,metadata)=>execFileSync(compiler,[wasm,target,code,metadata],{env,encoding:'utf8',windowsHide:true}));
 const records=[];
 for(const [number,file]of files.entries()){
  const unpack=path.join(work,String(number));removeUnder(work,unpack);
  const folder=await extractPackage(file,unpack),manifest=JSON.parse(fs.readFileSync(path.join(folder,'manifest.json'),'utf8'));
  const platform=platformOf(file,manifest),output=path.join(out,path.basename(file));
  if(!CLIENTS.has(manifest.id)){fs.copyFileSync(file,output);continue;}
  assert(manifest.component==='extension.wasm','Official clients must name extension.wasm');
  // Never trust unsigned native code handed over by a plugin builder.
  removeUnder(work,path.join(folder,'aot'));
  const selected=platform==='any'?Object.values(TARGETS):[TARGETS[platform]];
  assert(selected.every(Boolean),'Unsupported publication AOT target');
  for(const target of selected){
   const dir=path.join(folder,'aot',target);fs.mkdirSync(dir,{recursive:true});
   const metadata=path.join(dir,'manifest.json');
   compile(path.join(folder,'extension.wasm'),target,path.join(dir,'component.cwasm'),metadata);
   const value={...JSON.parse(fs.readFileSync(metadata,'utf8')),schema:1,id:manifest.id,version:manifest.version};
   assert(value.profile==='loog-wasmtime-50.0.1-portable-v1'&&value.target===target,'Unexpected AOT compiler profile or target');
   assert(value.wasm_sha256===fileHash(path.join(folder,'extension.wasm'))&&value.code_sha256===fileHash(path.join(dir,'component.cwasm')),'AOT compiler output digests do not match');
   const bytes=Buffer.from(JSON.stringify(value,null,2)+'\n');
   fs.writeFileSync(metadata,bytes);fs.writeFileSync(path.join(dir,'manifest.sig'),signIndex(Buffer.concat([DOMAIN,bytes]),privatePem,keyId));
   records.push({id:manifest.id,version:manifest.version,...value});
  }
  await createPackage(folder,output,manifest.id);
 }
 fs.writeFileSync(path.join(out,'aot-provenance.json'),JSON.stringify({schema:1,authority:keyId,profile:'loog-wasmtime-50.0.1-portable-v1',artifacts:records},null,2)+'\n');
 return records;
}
if(process.argv[1]&&path.resolve(process.argv[1])===fileURLToPath(import.meta.url)){
 const args=process.argv.slice(2),testAt=args.indexOf('--test-key');
 let privatePem=process.env.LOOG_INDEX_KEY,keyId,keys;
 if(testAt>=0){const file=args.splice(testAt,2)[1];privatePem=fs.readFileSync(file,'utf8');keyId='benchmark-only';keys={[keyId]:publicKeyOf(privatePem)};}
 else {const config=readToml(path.join(ROOT,'loog-ext.toml'));keyId=config.index['key-id'];keys=config.index.keys;}
 assert(privatePem,'LOOG_INDEX_KEY is required in the trusted publisher');
 const outAt=args.indexOf('--out');assert(outAt>=0,'--out is required');const out=path.resolve(args.splice(outAt,2)[1]);
 const compiler=process.env.LOOG_AOT_COMPILER??path.join(ROOT,'tools/aot-compiler/target/release/loog-aot-compiler'+(process.platform==='win32'?'.exe':''));
 const records=await prepareAot(packageFiles(args),{compiler,privatePem,keyId,keys,out,work:path.join(ROOT,'.work/aot-publication')});
 console.log(`Prepared ${records.length} signed AOT components`);
}
