// The core's own thread pushing a line a second: the bridge, both ways. Its own
// component so a tick re-renders one Text instead of the whole page.
//
// It reads as an instrument readout — a label, a live dot and the number — and
// the core's "up 9s" loses its prefix, because the label above already says
// what is being counted.

import { Event } from '../../core/RustCore';
import { onCoreEvent } from '../../core/events';

@ComponentV2
export struct Heartbeat {
  @Local private line: string = '';
  private off: () => void = (): void => {};

  aboutToAppear(): void {
    this.off = onCoreEvent('text', (e: Event): void => {
      this.line = e.text ?? '';
    });
  }

  aboutToDisappear(): void {
    this.off();
  }

  /** "up 9s" from the core becomes "9s": the label carries the "up". */
  private uptime(): string {
    if (this.line.length === 0) {
      return '—';
    }
    return this.line.startsWith('up ') ? this.line.substring(3) : this.line;
  }

  build() {
    Column({ space: 4 }) {
      Text('Core uptime')
        .fontSize(11)
        .fontColor($r('app.color.text_muted'))
      Row({ space: 6 }) {
        // The one thing on the page that is actually alive right now, so it is
        // the one thing allowed the bright accent.
        Circle()
          .width(6)
          .height(6)
          .fill($r('app.color.accent_live'))
        Text(this.uptime())
          .fontSize(13)
          .fontColor($r('app.color.text_primary'))
          .fontFeature('"tnum" 1')
      }
      .alignItems(VerticalAlign.Center)
    }
    .alignItems(HorizontalAlign.End)
  }
}
