// What a call across the bridge costs when there is nothing in it — the floor
// under every synchronous "ArkTS asks the core something" on the UI thread.
// Three measurements, because three different things are in the way:
//
// - int back (remaining): NAPI call, cgo jump, C.int back. Pure overhead.
// - string back (todos): the same plus CString malloc/free, the copy into a JS
//   string, and JSON.parse. This is what a query result costs today.
// - 300 KB in (analyze): overhead plus the buffer copy. The core reports its own
//   maths time (micros), so total minus maths is the bridge's share.
//
// A few microseconds means a database query can be a synchronous call; a few
// hundred means it has to go through the event channel instead.

import { systemDateTime } from '@kit.BasicServicesKit';
import { rustcore } from '../../core/RustCore';

@ComponentV2
export struct BridgeCard {
  @Local private intLine: string = '';
  @Local private jsonLine: string = '';
  @Local private pixelsLine: string = '';
  @Local private note: string = '';

  /** Nanoseconds since boot: Date.now() is far too coarse for one call. */
  private nowNs(): number {
    return systemDateTime.getUptime(systemDateTime.TimeType.ACTIVE, true);
  }

  private run(): void {
    const rounds = 20000;
    const warmup = 2000;
    for (let i = 0; i < warmup; i++) {
      rustcore.remaining();
      rustcore.todos();
    }

    // An int back: nothing allocated on either side.
    let start = this.nowNs();
    for (let i = 0; i < rounds; i++) {
      rustcore.remaining();
    }
    const intNs = (this.nowNs() - start) / rounds;

    // A JSON string back, parsed: what a result actually costs today.
    start = this.nowNs();
    for (let i = 0; i < rounds; i++) {
      rustcore.todos();
    }
    const strNs = (this.nowNs() - start) / rounds;

    // 300 KB in: overhead plus the copy, with the core's clock to separate the
    // maths from the crossing.
    const w = 240;
    const h = 320;
    const px = new ArrayBuffer(w * h * 4);
    const u8 = new Uint8Array(px);
    for (let i = 0; i < u8.length; i += 4) {
      const p = i >> 2;
      u8[i] = p & 0xFF;
      u8[i + 1] = (p >> 8) & 0xFF;
      u8[i + 2] = 128;
      u8[i + 3] = 255;
    }
    const pixRounds = 20;
    let micros = 0;
    start = this.nowNs();
    for (let i = 0; i < pixRounds; i++) {
      const res = rustcore.analyze(px, w, h, 0);
      micros = res.image?.micros ?? 0;
    }
    const pixNs = (this.nowNs() - start) / pixRounds;
    const mathNs = micros * 1000;

    const us = (ns: number): string => (ns / 1000).toFixed(1);
    this.intLine = `${us(intNs)} µs`;
    this.jsonLine = `${us(strNs)} µs`;
    this.pixelsLine = `${us(pixNs)} µs`;
    this.note = `maths ${us(mathNs)} µs · bridge ${us(pixNs - mathNs)} µs`;
    // Same numbers in the log: a screenshot is evidence, text is easier to cite.
    console.info(`rustkit-bench bridge: int ${this.intLine} · json+parse ${this.jsonLine}` +
      ` · ${this.note}`);
  }

  build() {
    Column({ space: 12 }) {
      Row({ space: 8 }) {
        Text('Bridge')
          .fontSize(15)
          .fontWeight(FontWeight.Medium)
          .fontColor($r('app.color.text_primary'))
        Blank()
        Button('Measure')
          .type(ButtonType.Normal)
          .height(32)
          .fontSize(13)
          .borderRadius(8)
          .padding({ left: 16, right: 16 })
          .backgroundColor($r('app.color.surface_raised'))
          .fontColor($r('app.color.text_primary'))
          .onClick(() => {
            this.run();
          })
      }
      .width('100%')
      .alignItems(VerticalAlign.Center)

      if (this.intLine.length > 0) {
        Column({ space: 8 }) {
          Row({ space: 12 }) {
            Text('int back')
              .fontSize(11)
              .fontColor($r('app.color.text_muted'))
              .width(72)
            Text(this.intLine)
              .fontSize(13)
              .fontColor($r('app.color.text_primary'))
              .fontFeature('"tnum" 1')
          }
          .width('100%')
          Row({ space: 12 }) {
            Text('JSON back')
              .fontSize(11)
              .fontColor($r('app.color.text_muted'))
              .width(72)
            Text(this.jsonLine)
              .fontSize(13)
              .fontColor($r('app.color.text_primary'))
              .fontFeature('"tnum" 1')
          }
          .width('100%')
          Row({ space: 12 }) {
            Text('300 KB in')
              .fontSize(11)
              .fontColor($r('app.color.text_muted'))
              .width(72)
            Text(this.pixelsLine)
              .fontSize(13)
              .fontColor($r('app.color.text_primary'))
              .fontFeature('"tnum" 1')
          }
          .width('100%')
        }
        .alignItems(HorizontalAlign.Start)
        .width('100%')
      } else {
        Text('An empty call, a parsed one, and a large buffer')
          .fontSize(13)
          .fontColor($r('app.color.text_secondary'))
          .width('100%')
      }

      if (this.note.length > 0) {
        Text(this.note)
          .fontSize(12)
          .fontColor($r('app.color.text_muted'))
          .fontFeature('"tnum" 1')
          .maxLines(2)
          .width('100%')
      }
    }
    .alignItems(HorizontalAlign.Start)
    .width('100%')
    .padding(16)
    .backgroundColor($r('app.color.surface'))
    .borderRadius(12)
  }
}
