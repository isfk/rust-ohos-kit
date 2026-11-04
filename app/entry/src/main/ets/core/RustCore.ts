// The Rust core, typed: one place that knows the wire format is JSON, so the
// UI only sees objects. Everything here is a thin wrapper over librustcore.so
// (see entry/src/main/cpp/types/librustcore/index.d.ts).

import core from 'librustcore.so';
import { util } from '@kit.ArkTS';

/** One row of the list (core.Todo). */
export interface Todo {
  id: number;
  title: string;
  done: boolean;
}

/** One row of a big list (core.Row). */
export interface Row {
  id: number;
  title: string;
  done: boolean;
}

/** What the core makes of the accelerometer (core.LevelReading). */
export interface LevelReading {
  pitch: number;
  roll: number;
  rate: number;
  x: number;
  y: number;
  z: number;
}

/** What the core makes of a file (core.FileInfo). */
export interface FileInfo {
  name: string;
  bytes: number;
  kind: string;
  sha256: string;
}

/** A file inspection's answer: the file, or why not. */
export interface InspectResult {
  ok: boolean;
  file?: FileInfo;
  error?: string;
}

/** The track after a fix (core.GeoPoint). */
export interface GeoPoint {
  lat: number;
  lon: number;
  accuracy: number;
  fixes: number;
  /** Metres travelled since the first fix. */
  moved: number;
  /** Degrees from the first fix, 0 = north. */
  bearing: number;
  /** Metres per second, smoothed. */
  speed: number;
  seconds: number;
}

/** A location fix's answer: the track, or why not. */
export interface GeoResult {
  ok: boolean;
  point?: GeoPoint;
  error?: string;
}

/** One of the colours that make up a picture (core.ColorShare). */
export interface ColorShare {
  hex: string;
  share: number;
}

/** A picture, in numbers (core.Analysis). */
export interface ImageAnalysis {
  width: number;
  height: number;
  meanLuma: number;
  meanRgb: number[];
  /** Eight percentages, darkest to brightest: the bars the UI draws. */
  histo: number[];
  /** 8×8 brightness 0..100, row by row: the grid the UI draws. */
  grid: number[];
  colors: ColorShare[];
  /** How long the core's maths took, in microseconds. */
  micros: number;
}

/** An analysis' answer: the picture, or why not. */
export interface AnalyzeResult {
  ok: boolean;
  image?: ImageAnalysis;
  error?: string;
}

/** What the core tells the UI (core.Event): a new list, or a line of text. */
export interface Event {
  kind: string;
  todos?: Todo[];
  text?: string;
  level?: LevelReading;
}

/** One row of the core's database. */
export interface DbRow {
  id: number;
  title: string;
  done: boolean;
}

/** A timed database operation: what it did, and how long the store itself took. */
export interface DbResult {
  ok: boolean;
  op: string;
  rows: number;
  micros: number;
  error?: string;
}

/** Where the core's database file lives. */
export interface DbOpenResult {
  ok: boolean;
  path: string;
  error?: string;
}

/** One page out of the core's database. */
export interface DbPageResult {
  ok: boolean;
  rows: DbRow[];
  count: number;
  micros: number;
  error?: string;
}

function todosOf(json: string): Todo[] {
  try {
    const e = JSON.parse(json) as Event;
    return e.todos ?? [];
  } catch (err) {
    return [];
  }
}

export class RustCore {
  /**
   * Takes the core's events. Call it before start(): the core pushes the
   * list as soon as it has read it.
   */
  listen(fn: (e: Event) => void): void {
    core.listen((json: string) => {
      try {
        fn(JSON.parse(json) as Event);
      } catch (err) {
        // a malformed event isn't worth taking the UI down for
      }
    });
  }

  /** Starts the core: reads the list from filesDir, starts its ticker. */
  start(filesDir: string): void {
    core.start(filesDir);
  }

  stop(): void {
    core.stop();
  }

  todos(): Todo[] {
    return todosOf(core.snapshot());
  }

  add(title: string): Todo[] {
    return todosOf(core.add(title));
  }

  toggle(id: number): Todo[] {
    return todosOf(core.toggle(id));
  }

  remove(id: number): Todo[] {
    return todosOf(core.remove(id));
  }

  remaining(): number {
    return core.remaining();
  }

  /**
   * Feeds the core one location fix; it answers with the track so far. The
   * platform's location Kit measured where we are — distance, bearing and
   * speed are the core's arithmetic.
   */
  location(lat: number, lon: number, accuracy: number): GeoResult {
    try {
      return JSON.parse(core.location(lat, lon, accuracy)) as GeoResult;
    } catch (err) {
      return { ok: false, error: `${err}` };
    }
  }

  /** Forgets the track: a new session starts from zero. */
  locationReset(): void {
    core.locationReset();
  }

  /**
   * Describes decoded pixels: brightness, histogram, the colours that make it
   * up, and where the light falls. The decoding is the platform's job (its
   * decoder reads HEIF, which the core has no business knowing about); the
   * arithmetic is the core's.
   */
  analyze(pixels: ArrayBuffer, width: number, height: number, stride: number = 0): AnalyzeResult {
    try {
      return JSON.parse(core.analyze(pixels, width, height, stride)) as AnalyzeResult;
    } catch (err) {
      return { ok: false, error: `${err}` };
    }
  }

  /**
   * Inspects a file the system opened for us (a photo from the picker): the
   * core reads it and reports its size, format and hash. Close the file here
   * — the core only borrows the descriptor.
   */
  inspect(fd: number, name: string): InspectResult {
    try {
      return JSON.parse(core.inspect(fd, name)) as InspectResult;
    } catch (err) {
      const out: InspectResult = { ok: false, error: 'bad answer from the core' };
      return out;
    }
  }

  /** How many rows the big list has. */
  rowsCount(): number {
    return core.rowsCount();
  }

  /**
   * A page of the big list, decoded from the compact form: the UI's own
   * parsing, of bytes the core laid out. No JSON, no intermediate strings.
   */
  rows(offset: number, limit: number): Row[] {
    const buf: ArrayBuffer = core.rows(offset, limit);
    return decodeRows(buf);
  }

  /** The same page as JSON (the comparison). */
  rowsJson(offset: number, limit: number): Row[] {
    try {
      const page = JSON.parse(core.rowsJson(offset, limit)) as RowsPage;
      return page.rows ?? [];
    } catch (err) {
      return [];
    }
  }

  /** The same page, still JSON: for measuring the parse on its own. */
  rowsJsonRaw(offset: number, limit: number): string {
    return core.rowsJson(offset, limit);
  }

  /** The same page, still bytes: for measuring the bridge on its own. */
  rowsRaw(offset: number, limit: number): ArrayBuffer {
    return core.rows(offset, limit);
  }

  /** Decodes the compact form. */
  decode(buf: ArrayBuffer): Row[] {
    return decodeRows(buf);
  }

  /** One accelerometer sample (m/s²): the sensor calls this ~50 times a second. */
  accel(x: number, y: number, z: number): void {
    core.accel(x, y, z);
  }

  /**
   * Opens the core's own SQLite file. The path is the app's sandbox: a Go
   * library in HarmonyOS has no environment to read it from.
   */
  dbOpen(filesDir: string): DbOpenResult {
    try {
      return JSON.parse(core.dbOpen(filesDir)) as DbOpenResult;
    } catch (err) {
      const out: DbOpenResult = { ok: false, path: '', error: `${err}` };
      return out;
    }
  }

  /** Replaces the core's table with n rows, in one transaction. */
  dbSeed(n: number): DbResult {
    try {
      return JSON.parse(core.dbSeed(n)) as DbResult;
    } catch (err) {
      const out: DbResult = { ok: false, op: 'seed', rows: 0, micros: 0, error: `${err}` };
      return out;
    }
  }

  /** A page of the core's table, in id order. */
  dbPage(offset: number, limit: number): DbPageResult {
    try {
      return JSON.parse(core.dbPage(offset, limit)) as DbPageResult;
    } catch (err) {
      const out: DbPageResult = { ok: false, rows: [], count: 0, micros: 0, error: `${err}` };
      return out;
    }
  }

  /** How many rows the core's table holds. */
  dbCount(): number {
    return core.dbCount();
  }

  /** Empties the core's table, keeping the file. */
  dbReset(): DbResult {
    try {
      return JSON.parse(core.dbReset()) as DbResult;
    } catch (err) {
      const out: DbResult = { ok: false, op: 'reset', rows: 0, micros: 0, error: `${err}` };
      return out;
    }
  }

  /** The app is on screen (true) or not. */
  lifecycle(foreground: boolean): void {
    core.lifecycle(foreground ? 1 : 0);
  }
}

interface RowsPage {
  rows?: Row[];
}

const utf8: util.TextDecoder = util.TextDecoder.create('utf-8');

function decodeRows(buf: ArrayBuffer): Row[] {
  const view = new DataView(buf);
  const count = view.getUint32(0, true);
  const out: Row[] = [];
  let off = 4;
  for (let i = 0; i < count; i++) {
    const id = view.getUint32(off, true);
    off += 4;
    const done = view.getUint8(off) === 1;
    off += 1;
    const len = view.getUint16(off, true);
    off += 2;
    const title = utf8.decodeToString(new Uint8Array(buf, off, len));
    off += len;
    const row: Row = { id: id, title: title, done: done };
    out.push(row);
  }
  return out;
}

export const rustcore: RustCore = new RustCore();
