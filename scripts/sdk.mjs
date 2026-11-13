import {fileURLToPath} from 'node:url';
import {prepareSdk} from './lib/sdk.mjs';

console.log(prepareSdk(fileURLToPath(new URL('../', import.meta.url)), process.argv[2] ?? process.env.LOOG_SOURCE));
