import test from 'node:test';import assert from 'node:assert/strict';import fs from 'node:fs';import path from 'node:path';
import {createHash} from 'node:crypto';
import {workspace,write} from './fixtures.mjs';
import {createPackage,extractPackage} from '../lib/archive.mjs';
import {generateKeyPair,verifyIndex} from '../lib/sign.mjs';
import {prepareAot,TARGETS} from '../prepare-aot.mjs';
import {emitCorpus,officialInputs} from '../aot-contract.mjs';
const hash=b=>createHash('sha256').update(b).digest('hex');
test('the trusted publisher discards handed-over native code and binds generated code to WASM and package identity',async t=>{
 const root=workspace(t),id='loog.lsp.gopls',folder=path.join(root,id),target=TARGETS['windows-x86_64'];
 write(folder,'manifest.json',{id,version:'0.2.4',component:'extension.wasm'});write(folder,'extension.wasm','test-wasm');write(folder,'aot/evil.cwasm','untrusted');
 const input=path.join(root,id+'-0.2.4-windows-x86_64.loogext');await createPackage(folder,input,id);
 const key=generateKeyPair(),out=path.join(root,'out');let count=0;
 await prepareAot([input],{privatePem:key.privatePem,keyId:'test',keys:{test:key.publicRaw},out,work:path.join(root,'work'),compile:(wasm,triple,code,metadata)=>{
  assert.equal(triple,target);assert.ok(!fs.existsSync(path.join(path.dirname(wasm),'aot/evil.cwasm')));count++;
  const bytes=Buffer.from('compiler-owned-code');fs.writeFileSync(code,bytes);fs.writeFileSync(metadata,JSON.stringify({profile:'loog-wasmtime-50.0.1-portable-v1',target:triple,compatibility:'test',wasm_sha256:hash(fs.readFileSync(wasm)),code_sha256:hash(bytes)}));
 }});
 const unpacked=await extractPackage(path.join(out,path.basename(input)),path.join(root,'verified'),id),dir=path.join(unpacked,'aot',target);
 const bytes=fs.readFileSync(path.join(dir,'manifest.json'));
 assert.equal(count,1);assert.equal(JSON.parse(bytes).version,'0.2.4');
 assert.ok(verifyIndex(Buffer.concat([Buffer.from('loog.extension.aot.v1\0'),bytes]),fs.readFileSync(path.join(dir,'manifest.sig'),'utf8'),{test:key.publicRaw}).ok);
 assert.ok(!verifyIndex(bytes,fs.readFileSync(path.join(dir,'manifest.sig'),'utf8'),{test:key.publicRaw}).ok);
});
test('a mismatched publication authority is refused before compilation',async t=>{
 const root=workspace(t),a=generateKeyPair(),b=generateKeyPair();let compiled=false;
 await assert.rejects(prepareAot([],{privatePem:a.privatePem,keyId:'test',keys:{test:b.publicRaw},out:path.join(root,'out'),work:path.join(root,'work'),compile:()=>compiled=true}),/publication authority/);
 assert.equal(compiled,false);
});
test('AOT signing remains in the publisher after trusted compilation and before immutable intake',()=>{
 const workflow=fs.readFileSync(new URL('../../.github/workflows/publish.yml',import.meta.url),'utf8');
 assert.ok(workflow.indexOf('Build the trusted cross-platform AOT compiler')<workflow.indexOf('run: node scripts/prepare-aot.mjs'));
 assert.ok(workflow.indexOf('run: node scripts/prepare-aot.mjs')<workflow.indexOf('release intake .work/aot-packages'));
 const source=fs.readFileSync(new URL('../prepare-aot.mjs',import.meta.url),'utf8');assert.match(source,/delete env.LOOG_INDEX_KEY/);
 assert.equal(Object.keys(TARGETS).length,6);
});

test('native interoperability gates publication and uses immutable compiler artifact IDs',()=>{
 const workflow=fs.readFileSync(new URL('../../.github/workflows/publish.yml',import.meta.url),'utf8');
 assert.match(workflow,/publish:\s+needs: \[aot-corpus, aot-native\]/);
 assert.match(workflow,/artifact-ids:.*needs.aot-corpus.outputs.artifact-id/);
 // ID-based downloads otherwise nest even a single artifact under its name.
 assert.match(workflow,/artifact-ids:[^\n]+\n\s+path: \.work\/aot-corpus\n\s+merge-multiple: true/);
 assert.match(workflow,/needs.aot-native.result == 'success'/);
 assert.match(workflow,/needs.aot-native.result == 'skipped'.*needs.aot-corpus.outputs.count == '0'/);
 assert.match(workflow,/inputs.publish && github.ref == 'refs\/heads\/main' && github.event_name != 'pull_request'/);
 assert.match(workflow,/merge-base --is-ancestor "\$SDK_REVISION" HEAD/);
 for(const target of Object.values(TARGETS))assert.ok(workflow.includes('target: '+target));
});

test('conformance refuses divergent platform WASM before certifying an ABI corpus',async t=>{
 const root=workspace(t),id='loog.lsp.gopls',files=[];
 for(const [platform,wasm]of [['windows-x86_64','first'],['linux-x86_64','different']]){
  const folder=path.join(root,platform,id);write(folder,'manifest.json',{id,version:'0.2.4',component:'extension.wasm'});write(folder,'extension.wasm',wasm);
  const file=path.join(root,id+'-0.2.4-'+platform+'.loogext');await createPackage(folder,file,id);files.push(file);
 }
 assert.equal(officialInputs(files).length,2);
 await assert.rejects(emitCorpus(files,path.join(root,'corpus'),{compiler:'must-not-run',work:path.join(root,'work')}),/Platform build inputs disagree/);
 assert.ok(!fs.existsSync(path.join(root,'corpus/public-key.txt')));
});
