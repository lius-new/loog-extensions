// Compile the same private Rust source independently for each extension package.
import fs from 'node:fs';
import path from 'node:path';
import {execFileSync} from 'node:child_process';

export function buildClient(directory, {checkOnly = false} = {}) {
  const loog=path.resolve(process.env.LOOG_SOURCE??path.join(directory,'../../../..'));
  // The clients say how their service is doing (loog:service/status) and what it
  // found in files that are not open (loog:service/findings); Loog draws both.
  const world=fs.existsSync(path.join(loog,'wit/loog-extension/world.wit'))?fs.readFileSync(path.join(loog,'wit/loog-extension/world.wit'),'utf8'):'';
  if(!fs.existsSync(path.join(loog,'wit/loog-extension/deps/service/world.wit')) ||
     !world.includes('import loog:service/status@0.1.0;') || !world.includes('import loog:service/findings@0.1.0;'))
    throw new Error('These clients require loog:service/status@0.1.0 and loog:service/findings@0.1.0; set LOOG_SOURCE to a Loog checkout that has them');
  const crate=path.join(directory,'rust'),manifest=path.join(crate,'Cargo.toml');
  const name=/^name\s*=\s*"([^"]+)"/m.exec(fs.readFileSync(manifest,'utf8'))?.[1];
  if(!name)throw new Error('Missing Rust package name');
  const env={...process.env,LOOG_SOURCE:loog};
  const run=args=>execFileSync('cargo',args,{cwd:directory,env,stdio:'inherit',windowsHide:true});
  if(checkOnly){run(['test','--manifest-path',manifest,'--locked','--lib']);return;}
  run(['build','--manifest-path',manifest,'--locked','--release','--target','wasm32-unknown-unknown']);
  const target=process.env.CARGO_TARGET_DIR?path.resolve(directory,process.env.CARGO_TARGET_DIR):path.join(crate,'target');
  const output=path.join(directory,'.work/client/extension.wasm');fs.mkdirSync(path.dirname(output),{recursive:true});
  run(['run','--manifest-path',path.join(import.meta.dirname,'componentize/Cargo.toml'),'--locked','--release','--',
    path.join(target,'wasm32-unknown-unknown/release',name.replaceAll('-','_')+'.wasm'),output]);
  return output;
}

export function clientNotices(folder,directory,{source=path.resolve(directory,'../..')}={}){
  const licenses=path.join(folder,'licenses');fs.mkdirSync(licenses,{recursive:true});
  for(const name of ['LICENSE-MIT','LICENSE-APACHE'])fs.copyFileSync(path.join(source,name),path.join(licenses,`loog-client-${name}`));
  const manifest=path.join(directory,'rust/Cargo.toml');
  const metadata=JSON.parse(execFileSync('cargo',['metadata','--manifest-path',manifest,'--locked','--format-version','1','--filter-platform','wasm32-unknown-unknown'],{cwd:directory,encoding:'utf8',windowsHide:true}));
  const notices=[];
  for(const pkg of metadata.packages){
    if(path.resolve(pkg.manifest_path)===path.resolve(manifest))continue;
    const source=path.dirname(pkg.manifest_path),files=fs.readdirSync(source,{withFileTypes:true}).filter(entry=>entry.isFile()&&/^(LICENSE|COPYING|NOTICE)([.-]|$)/i.test(entry.name));
    if(!files.length)throw new Error(`Missing redistribution notices for ${pkg.name}`);
    const destination=path.join(licenses,'rust',`${pkg.name}-${pkg.version}`);fs.mkdirSync(destination,{recursive:true});
    for(const file of files)fs.copyFileSync(path.join(source,file.name),path.join(destination,file.name));
    notices.push(`${pkg.name} ${pkg.version}: ${pkg.license??'see included license'}; ${pkg.repository??'https://crates.io/crates/'+pkg.name}`);
  }
  fs.writeFileSync(path.join(licenses,'CLIENT-NOTICES.txt'),
    'Private Rust language client: MIT OR Apache-2.0.\nSource: https://github.com/lius-new/loog-extensions/tree/main/language-clients/rust\n'+
    'The component contains no JavaScript engine. Rust dependency licenses are included below licenses/rust.\n'+
    'Status icons: Material Design Icons by Pictogrammers (https://github.com/Templarian/MaterialDesign), Apache-2.0; see loog-client-LICENSE-APACHE.\n\n'+notices.sort().join('\n')+'\n');
}
