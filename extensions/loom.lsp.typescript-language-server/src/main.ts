import * as api from '@loom/extension';
import * as host from 'loom:extension/host@0.1.0';
import { activate } from '../../../language-clients/client.ts';
import manifest from '../manifest.json';

activate(api, host, {
  id: manifest.id, name: manifest.name, languages: ['javascript', 'typescript'],
  program: '${runtime.node}/node',
  args: ['${extension}/node_modules/typescript-language-server/lib/cli.mjs', '--stdio'],
  markers: ['tsconfig.json', 'jsconfig.json', 'package.json'],
  languageId: (language, variant) => variant === 'tsx' ? 'typescriptreact' : variant === 'jsx' ? 'javascriptreact' : language,
});
export const guest = api.guest;
