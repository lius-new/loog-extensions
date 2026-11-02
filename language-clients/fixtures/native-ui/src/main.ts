// Test entry of the same extension-owned client used by the official packages.
// Loom's editor regressions copy the resulting component as a test fixture.
import * as host from 'loom:extension/host@0.1.0';
import * as api from '@loom/extension';
import { activate } from '../../../client.ts';

activate(api, host, {
  id: 'example.language-fixture', name: 'Fixture', languages: ['rust'],
  program: '${extension}/fake_lsp',
  get args(): string[] {
    let options = JSON.parse(host.getSetting('example.language-fixture.options') ?? '{}');
    if (typeof options === 'string') options = JSON.parse(options);
    return options.args ?? [];
  },
  markers: [],
  languageId: language => language,
});
export const guest = api.guest;
