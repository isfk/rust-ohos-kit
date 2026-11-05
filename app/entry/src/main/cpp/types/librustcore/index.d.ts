/**
 * The Rust core, as ArkTS sees it (librustcore.so, built by cargo).
 *
 * Calls are synchronous and run on the main thread: keep them small. The
 * core's own threads talk back through listen().
 */

export interface Todo {
  id: number;
  title: string;
  done: boolean;
}

/** One row the core's database holds. */
export interface DbRow {
  id: number;
  title: string;
  done: boolean;
}

/** A timed database operation: what it did, and how long the store took. */
export interface DbResult {
  ok: boolean;
  op: string;
  rows: number;
  micros: number;
  error?: string;
}

/** Where the core's database file is. */
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

/** Calls the core's events: a new list, or a line of text. */
export function listen(fn: (json: string) => void): void;

/**
 * Starts the core: opens the database under filesDir, reads the list out of it,
 * starts its ticker. Without a usable filesDir the list works and does not
 * survive the session — the log at start says which of the two happened.
 */
export function start(filesDir: string): void;

/** Stops the core. Nothing is written here: every change was committed already. */
export function stop(): void;

/** The list, as JSON (a snapshot event). */
export function snapshot(): string;

/** Adds a todo; returns the list after it. */
export function add(title: string): string;

/** Turns a todo done, or not; returns the list after it. */
export function toggle(id: number): string;

/** Removes a todo; returns the list after it. */
export function remove(id: number): string;

/** How many are left to do. */
export function remaining(): number;

/** What a file the system opened is: `{ok, file}` or `{ok:false, error}`. */
export function inspect(fd: number, name: string): string;

/** One location fix in; the track so far out: `{ok, point}` or `{ok:false, error}`. */
export function location(lat: number, lon: number, accuracy: number): string;

/** Forgets the track (the UI stopped listening). */
export function locationReset(): void;

/**
 * What a decoded picture looks like: `{ok, image}` or `{ok:false, error}`.
 * pixels is RGBA_8888, width×height, row stride in bytes (0 = packed).
 */
export function analyze(pixels: ArrayBuffer, width: number, height: number, stride?: number): string;

/** The app moved on screen (1) or to the background (0). */
export function lifecycle(foreground: number): void;

/** How many rows the big list has. */
export function rowsCount(): number;

/**
 * A page of the big list, in the compact form: one ArrayBuffer, no parsing.
 * u32 count, then per row u32 id, u8 done, u16 title length, title bytes.
 */
export function rows(offset: number, limit: number): ArrayBuffer;

/** The same page as JSON, to measure the compact form against. */
export function rowsJson(offset: number, limit: number): string;

/**
 * One accelerometer sample, in m/s² (gravity included). Call it from the
 * sensor's callback: it does no JavaScript work and allocates nothing.
 */
export function accel(x: number, y: number, z: number): void;

/**
 * Opens the core's own SQLite file under filesDir — the list and the 20k
 * benchmark rows live in this one file. Idempotent, and `start` has already
 * opened it, so this is mostly a way to ask where it is. Two writers on one file
 * is SQLITE_BUSY waiting to happen, so the file is the core's alone: the
 * platform's relationalStore owns the one it manages.
 */
export function dbOpen(filesDir: string): string;

/** Replaces the benchmark table with n rows, in one transaction. */
export function dbSeed(n: number): string;

/** A page of the benchmark table, in id order (JSON). */
export function dbPage(offset: number, limit: number): string;

/** How many rows the benchmark table holds (0 before it is open). */
export function dbCount(): number;

/** Empties the benchmark table, keeping the file (the list is untouched). */
export function dbReset(): string;
