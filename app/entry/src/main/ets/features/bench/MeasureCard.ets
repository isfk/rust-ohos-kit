// The measurement card for the wire format: the same 2000 rows twice, once as
// JSON (bytes → string → objects) and once in the compact form (bytes →
// objects). This is what the binary path is for, and what README's table says.
//
// Three readout rows instead of one long line: the two formats sit in the same
// column so the eye compares them, and the wire cost has its own row because it
// is not the same question.

import { rustcore, Row } from '../../core/RustCore';

@ComponentV2
export struct MeasureCard {
  @Local private jsonLine: string = '';
  @Local private binLine: string = '';
  @Local private wireLine: string = '';
  @Local private note: string = '';

  private run(): void {
    const page = 2000;
    const rounds = 10;
    let rows: Row[] = [];

    let start = Date.now();
    for (let i = 0; i < rounds; i++) {
      rows = rustcore.rowsJson(0, page);
    }
    const jsonMs = (Date.now() - start) / rounds;
    const jsonBytes = rustcore.rowsJsonRaw(0, page).length;

    start = Date.now();
    for (let i = 0; i < rounds; i++) {
      rows = rustcore.rows(0, page);
    }
    const binMs = (Date.now() - start) / rounds;
    const binBytes = rustcore.rowsRaw(0, page).byteLength;

    // And the wire on its own: no decoding at all, which is what a list that
    // only decodes the rows it shows actually pays.
    start = Date.now();
    for (let i = 0; i < rounds; i++) {
      rustcore.rowsRaw(0, page);
    }
    const wireMs = (Date.now() - start) / rounds;

    const kb = 1024;
    this.jsonLine = `${jsonMs.toFixed(1)} ms · ${(jsonBytes / kb).toFixed(0)} KB`;
    this.binLine = `${binMs.toFixed(1)} ms · ${(binBytes / kb).toFixed(0)} KB`;
    this.wireLine = `${wireMs.toFixed(1)} ms`;
    this.note = `${rustcore.rowsCount()} rows · e.g. ${rows.length > 0 ? rows[0].title : '?'}`;
    console.info(`rustkit-bench wire: ${this.note} · json ${this.jsonLine} · binary ${this.binLine}` +
      ` · wire ${this.wireLine}`);
  }

  build() {
    Column({ space: 12 }) {
      Row({ space: 8 }) {
        Text('Wire format')
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

      if (this.jsonLine.length > 0) {
        Column({ space: 8 }) {
          Row({ space: 12 }) {
            Text('JSON')
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
            Text('Binary')
              .fontSize(11)
              .fontColor($r('app.color.text_muted'))
              .width(72)
            Text(this.binLine)
              .fontSize(13)
              .fontColor($r('app.color.text_primary'))
              .fontFeature('"tnum" 1')
          }
          .width('100%')
          Row({ space: 12 }) {
            Text('Wire only')
              .fontSize(11)
              .fontColor($r('app.color.text_muted'))
              .width(72)
            Text(this.wireLine)
              .fontSize(13)
              .fontColor($r('app.color.text_primary'))
              .fontFeature('"tnum" 1')
          }
          .width('100%')
        }
        .alignItems(HorizontalAlign.Start)
        .width('100%')
      } else {
        Text('2000 rows through both paths')
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
