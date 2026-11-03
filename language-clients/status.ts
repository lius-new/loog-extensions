// Presentation policy of these language clients, compiled into each plugin.
// Loom receives ordinary row/text descriptions, not language-service states.
import type { Json } from './protocol.ts';

export type StatusState = 'ready' | 'busy' | 'stopped' | 'error';
export interface Status { state: StatusState; detail?: string }
const colors: Record<StatusState, number> = {
  ready: 0x10b981, busy: 0x818cf8, stopped: 0x71717a, error: 0xf43f5e,
};
const words: Record<StatusState, string> = {
  ready: 'Ready', busy: 'Working', stopped: 'Stopped', error: 'Error',
};
export const statusPriority: Record<StatusState, number> = {
  stopped: 0, ready: 1, busy: 2, error: 3,
};
function compact(text: string, limit: number): string {
  const characters = Array.from(text.replace(/\s+/g, ' ').trim());
  return characters.length > limit ? characters.slice(0, limit - 1).join('') + '…' : characters.join('');
}
export function presentation(name: string, status: Status): { text: string; label: string; background: number } {
  const detail = compact(status.detail ?? '', 80);
  const message = status.state === 'ready' ? name
    : status.state === 'busy' ? `${name}: ${detail || words.busy}`
    : `${name}: ${words[status.state]}${status.state === 'error' && detail ? ` · ${detail}` : ''}`;
  return {
    text: compact(message, 96),
    label: compact(status.state === 'ready' ? `${name}: ${words.ready}` : message, 256),
    background: colors[status.state],
  };
}

export class StatusWidget {
  private root: Json;
  private text: Json;
  private disposed = false;
  constructor(private readonly ui: Json, private readonly name: string, private readonly log: (message: string) => void) {}
  update(status: Status): void {
    if (this.disposed) return;
    const display = presentation(this.name, status);
    try {
      if (!this.root) {
        // Rows and text have no click handlers, hover styles, or pointer cursor.
        // Width is measured from the content; spacers supply horizontal padding.
        const root = this.ui.createElement(this.ui.row({ id: 'language-status', label: display.label, background: display.background, height: 24 }));
        try {
          this.ui.create(root, this.ui.text('', { id: 'language-status-leading', width: 10, height: 24 }));
          const text = this.ui.create(root, this.ui.text(display.text, { id: 'language-status-text', foreground: 0xffffff, height: 24 }));
          this.ui.create(root, this.ui.text('', { id: 'language-status-trailing', width: 10, height: 24 }));
          this.ui.insertBefore(this.ui.target('statusbar.cursor-position'), root, { order: 100, whenMissing: 'hide' });
          this.root = root; this.text = text;
        } catch (error) { root.remove(); throw error; }
      } else {
        this.root.update({ background: display.background, label: display.label });
        this.text.update({ text: display.text });
      }
    } catch (error) { this.log(`Status widget: ${error}`); }
  }
  dispose(): void {
    this.disposed = true;
    this.root?.remove();
    this.root = this.text = undefined;
  }
}
