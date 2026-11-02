import * as api from '@loom/extension';
import * as host from 'loom:extension/host@0.1.0';
import { activate } from '../../../language-clients/client.ts';

activate(api, host, {
  id: 'loom.lsp.gopls', name: 'gopls', languages: ['go'],
  program: '${extension}/gopls', args: [], markers: ['go.work', 'go.mod'],
  languageId: () => 'go',
});
export const guest = api.guest;
