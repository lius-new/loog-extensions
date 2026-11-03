import * as api from '@loom/extension';
import * as host from 'loom:extension/host@0.1.0';
import { activate } from '../../../language-clients/client.ts';
import manifest from '../manifest.json';

activate(api, host, {
  id: manifest.id, name: manifest.name, languages: ['rust'],
  program: '${extension}/rust-analyzer', args: [],
  markers: ['Cargo.toml', 'rust-project.json'], outermost: true, fileFallback: true,
  languageId: () => 'rust',
});
export const guest = api.guest;
