import test from 'node:test';
import assert from 'node:assert/strict';
import { StatusWidget, presentation } from './status.ts';

test('long multiline server failures stay bounded and retain complete Unicode characters', () => {
  const value = presentation('My Plugin', { state: 'error', detail: ('失败😀\n下一行\t').repeat(1000) });
  assert.match(value.text, /^My Plugin: Error · /);
  assert.ok(Array.from(value.text).length <= 96);
  assert.ok(Array.from(value.label).length <= 256);
  assert.ok(!/[\n\r\t]/.test(value.text));
  assert.ok(value.text.isWellFormed());
  assert.equal(value.background, 0xf43f5e);
});

test('a failed placement removes its partial subtree and disposal prevents late recreation', () => {
  let creates = 0, removes = 0, failPlacement = true;
  const logs: string[] = [];
  const descriptions: any[] = [];
  const ui = {
    row: (value: any) => ({ ...value, kind: 'row' }),
    text: (text: string, value: any) => ({ ...value, text, kind: 'text' }),
    createElement(value: any) { creates++; descriptions.push(value); return { update() {}, remove() { removes++; } }; },
    create(_parent: any, value: any) { descriptions.push(value); return { update() {} }; },
    target: (publicId: string) => ({ publicId }),
    insertBefore() { if (failPlacement) throw new Error('placement failed'); },
  };
  const widget = new StatusWidget(ui, 'My Plugin', message => logs.push(message));
  widget.update({ state: 'busy', detail: 'Starting' });
  assert.equal(removes, 1);
  assert.match(logs[0]!, /placement failed/);
  failPlacement = false;
  widget.update({ state: 'ready' });
  assert.equal(creates, 2);
  assert.ok(descriptions.every(node => node.kind === 'row' || node.kind === 'text'));
  assert.equal(descriptions[0].width, undefined);
  widget.dispose();
  widget.update({ state: 'error', detail: 'late notification' });
  assert.equal(creates, 2);
  assert.equal(removes, 2);
});
