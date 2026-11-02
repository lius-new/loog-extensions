import * as api from '@loom/extension';
import * as host from 'loom:extension/host@0.1.0';
import { activate } from '../../../language-clients/client.ts';

activate(api, host, {
  id: 'loom.lsp.rust-analyzer', name: 'rust-analyzer', languages: ['rust'],
  program: '${extension}/rust-analyzer', args: [],
  markers: ['Cargo.toml', 'rust-project.json'], outermost: true, fileFallback: true,
  languageId: () => 'rust',
});
export const guest = api.guest;
