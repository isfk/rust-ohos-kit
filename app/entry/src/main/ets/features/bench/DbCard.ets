// The fourth comparison, and the one this file exists for: the same 20k rows
// written by two databases that are both SQLite, from two sides of the bridge.
//
//   Go side   — the core's own file (modernc.org/sqlite, pure Go, no cgo)
//   RDB side  — the platform's relationalStore, which owns the file it manages
//
// They are deliberately two files, not two table names in one: one file has
// one writer. Two handles on the same file is SQLITE_BUSY, and it shows up
// only when it is inconvenient.
//
// Both numbers include building the rows (Go builds titles inside its call,
// the platform's writer builds ValuesBuckets here), because that is what the
// app actually pays. The Go side also reports the store's own clock, so the
// same call says what the bridge cost without a second experiment.
//
// The two sides are named on the page rather than implied: "Go · seed 20k" and
// "RDB · seed 20k" were a decoder ring for anyone who had not read this file.

import { relationalStore } from '@kit.ArkData';
import { systemDateTime } from '@kit.BasicServicesKit';
import { rustcore } from '../../core/RustCore';

const SEED = 20000; // what ../bench/sqlite calls a bulk import
const PAGE = 2000; // one screenful, the size the README quotes
const CHUNK = 2000; // batchInsert takes an array; 2k objects at a time

@ComponentV2
export struct DbCard {
  @Local private status: string = 'not opened';
  @Local private goSeedLine: string = '';
  @Local private goPageLine: string = '';
  @Local private rdbSeedLine: string = '';
  @Local private rdbPageLine: string = '';
  /**
   * The core's row count. A state field and not `rustcore.dbCount()` inline in
   * the Text: a plain call in build() gives the renderer nothing to watch, so
   * the label kept saying 0 after a seed.
   */
  @Local private coreRows: number = 0;
  private rdb: relationalStore.RdbStore | undefined = undefined;

  aboutToAppear(): void {
    const opened = rustcore.dbOpen(getContext(this).filesDir);
    this.status = opened.ok ? 'Rust file ready' : `open failed: ${opened.error ?? 'unknown'}`;
    this.refreshCount();
  }

  /** Nanoseconds since boot: Date.now() can't see a single call. */
  private nowNs(): number {
    return systemDateTime.getUptime(systemDateTime.TimeType.ACTIVE, true);
  }

  private note(line: string): void {
    console.info(`rustkit-bench db: ${line}`);
  }

  private refreshCount(): void {
    this.coreRows = rustcore.dbCount();
  }

  /** One bulk import, one transaction — the shape that ../bench/sqlite says matters most. */
  private goSeed(): void {
    const start = this.nowNs();
    const res = rustcore.dbSeed(SEED);
    const total = (this.nowNs() - start) / 1e6;
    this.refreshCount();
    if (!res.ok) {
      this.goSeedLine = res.error ?? 'seed failed';
      return;
    }
    this.goSeedLine = `seed · ${total.toFixed(1)} ms (store ${(res.micros / 1000).toFixed(1)})` +
      ` · ${res.rows} rows`;
    this.note(`go seed ${SEED}: ${this.goSeedLine}`);
  }

  private goPage(): void {
    const start = this.nowNs();
    const res = rustcore.dbPage(0, PAGE);
    const total = (this.nowNs() - start) / 1e6;
    this.refreshCount();
    if (!res.ok) {
      this.goPageLine = res.error ?? 'page failed';
      return;
    }
    this.goPageLine = `page · ${total.toFixed(1)} ms (store ${(res.micros / 1000).toFixed(2)})` +
      ` · ${res.count} rows`;
    this.note(`go page ${PAGE}: ${this.goPageLine}`);
  }

  /** The platform's store, opened once and reused. */
  private async platform(): Promise<relationalStore.RdbStore> {
    const open = this.rdb;
    if (open !== undefined) {
      return open;
    }
    const config: relationalStore.StoreConfig = {
      name: 'bench.db',
      securityLevel: relationalStore.SecurityLevel.S1,
    };
    const store = await relationalStore.getRdbStore(getContext(this), config);
    await store.executeSql('CREATE TABLE IF NOT EXISTS bench (id INTEGER PRIMARY KEY, title TEXT NOT NULL, done INTEGER NOT NULL)');
    this.rdb = store;
    return store;
  }

  private async rdbSeed(): Promise<void> {
    const store = await this.platform();
    const start = this.nowNs();
    await store.executeSql('DELETE FROM bench');
    for (let off = 0; off < SEED; off += CHUNK) {
      const end = Math.min(off + CHUNK, SEED);
      const values: relationalStore.ValuesBucket[] = [];
      for (let i = off; i < end; i++) {
        const row: relationalStore.ValuesBucket = {
          'id': i,
          'title': `row ${i}`,
          'done': i % 2 === 0 ? 1 : 0,
        };
        values.push(row);
      }
      await store.batchInsert('bench', values);
    }
    const total = (this.nowNs() - start) / 1e6;
    this.rdbSeedLine = `seed · ${total.toFixed(1)} ms · ${SEED} rows in ${SEED / CHUNK} batches`;
    this.note(`rdb seed ${SEED}: ${this.rdbSeedLine}`);
  }

  private async rdbPage(): Promise<void> {
    const store = await this.platform();
    const start = this.nowNs();
    const set = await store.querySql('SELECT id, title, done FROM bench ORDER BY id LIMIT ? OFFSET ?', [PAGE, 0]);
    let n = 0;
    while (set.goToNextRow()) {
      set.getLong(0);
      set.getString(1);
      set.getLong(2);
      n++;
    }
    set.close();
    const total = (this.nowNs() - start) / 1e6;
    this.rdbPageLine = `page · ${total.toFixed(1)} ms · ${n} rows`;
    this.note(`rdb page ${PAGE}: ${this.rdbPageLine}`);
  }

  private async resetBoth(): Promise<void> {
    rustcore.dbReset();
    const store = await this.platform();
    await store.executeSql('DELETE FROM bench');
    this.goSeedLine = '';
    this.goPageLine = '';
    this.rdbSeedLine = '';
    this.rdbPageLine = '';
    this.refreshCount();
    this.status = 'both empty';
  }

  build() {
    Column({ space: 12 }) {
      Row({ space: 8 }) {
        Text('SQLite')
          .fontSize(15)
          .fontWeight(FontWeight.Medium)
          .fontColor($r('app.color.text_primary'))
        Blank()
        Text(this.status)
          .fontSize(12)
          .fontColor($r('app.color.text_muted'))
          .layoutWeight(1)
          .maxLines(2)
          .textAlign(TextAlign.End)
          .textOverflow({ overflow: TextOverflow.Ellipsis })
      }
      .width('100%')
      .alignItems(VerticalAlign.Center)

      // Go's own file, through the bridge: one transaction, one statement.
      Text('Rust core')
        .fontSize(11)
        .fontColor($r('app.color.text_muted'))
      Row({ space: 8 }) {
        Button('Seed 20k')
          .type(ButtonType.Normal)
          .height(32)
          .fontSize(13)
          .borderRadius(8)
          .padding({ left: 12, right: 12 })
          .backgroundColor($r('app.color.surface_raised'))
          .fontColor($r('app.color.text_primary'))
          .onClick(() => {
            this.goSeed();
          })
        Button('Page 2k')
          .type(ButtonType.Normal)
          .height(32)
          .fontSize(13)
          .borderRadius(8)
          .padding({ left: 12, right: 12 })
          .backgroundColor($r('app.color.surface_raised'))
          .fontColor($r('app.color.text_primary'))
          .onClick(() => {
            this.goPage();
          })
      }
      .width('100%')
      if (this.goSeedLine.length > 0) {
        Text(this.goSeedLine)
          .fontSize(12)
          .fontColor($r('app.color.text_secondary'))
          .fontFeature('"tnum" 1')
          .maxLines(2)
          .width('100%')
      }
      if (this.goPageLine.length > 0) {
        Text(this.goPageLine)
          .fontSize(12)
          .fontColor($r('app.color.text_secondary'))
          .fontFeature('"tnum" 1')
          .maxLines(2)
          .width('100%')
      }

      // The platform's file, the same rows, from the other side. No rule
      // between the two: the label already says a new block starts, and a line
      // on top of the spacing on top of a label is one break said three times.
      Text('Platform store')
        .fontSize(11)
        .fontColor($r('app.color.text_muted'))
        .padding({ top: 4 })
      Row({ space: 8 }) {
        Button('Seed 20k')
          .type(ButtonType.Normal)
          .height(32)
          .fontSize(13)
          .borderRadius(8)
          .padding({ left: 12, right: 12 })
          .backgroundColor($r('app.color.surface_raised'))
          .fontColor($r('app.color.text_primary'))
          .onClick(() => {
            this.rdbSeed();
          })
        Button('Page 2k')
          .type(ButtonType.Normal)
          .height(32)
          .fontSize(13)
          .borderRadius(8)
          .padding({ left: 12, right: 12 })
          .backgroundColor($r('app.color.surface_raised'))
          .fontColor($r('app.color.text_primary'))
          .onClick(() => {
            this.rdbPage();
          })
      }
      .width('100%')
      if (this.rdbSeedLine.length > 0) {
        Text(this.rdbSeedLine)
          .fontSize(12)
          .fontColor($r('app.color.text_secondary'))
          .fontFeature('"tnum" 1')
          .maxLines(2)
          .width('100%')
      }
      if (this.rdbPageLine.length > 0) {
        Text(this.rdbPageLine)
          .fontSize(12)
          .fontColor($r('app.color.text_secondary'))
          .fontFeature('"tnum" 1')
          .maxLines(2)
          .width('100%')
      }

      Row({ space: 12 }) {
        Button('Reset both')
          .type(ButtonType.Normal)
          .height(32)
          .fontSize(13)
          .borderRadius(8)
          .padding({ left: 0, right: 12 })
          .backgroundColor('#00000000')
          .fontColor($r('app.color.danger'))
          .onClick(() => {
            this.resetBoth();
          })
        Blank()
        Text(`${this.coreRows} rows in the core's file`)
          .fontSize(12)
          .fontColor($r('app.color.text_muted'))
          .fontFeature('"tnum" 1')
      }
      .width('100%')
      .alignItems(VerticalAlign.Center)
    }
    .alignItems(HorizontalAlign.Start)
    .width('100%')
    .padding(16)
    .backgroundColor($r('app.color.surface'))
    .borderRadius(12)
  }
}
