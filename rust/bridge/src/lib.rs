//! The bridge: the only file here that knows a phone exists.
//!
//! It moves bytes and nothing else. Every decision — what a todo is, whether a
//! fix counts as walking, how bright a photo is — happened in `rustkit-core`
//! before it got here.
//!
//! The ABI is the Go version's, function for function: a `char *` of JSON that
//! the core allocates and [`rustkit_free`] releases, and one callback for events
//! that travel the other way. That is deliberate. If the seam really is a seam,
//! the C file on the other side should not be able to tell which language built
//! the library it is loading.
//!
//! The wire format and SQLite implementation live alongside the bridge: the
//! former keeps the NAPI surface small, and the latter is isolated in
//! `rustkit-store` because `rusqlite`'s bundled SQLite needs a C compiler.

use std::ffi::{c_char, CStr, CString};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use rustkit_core::file;
use rustkit_core::geo::{GeoFix, Track};
use rustkit_core::image;
use rustkit_core::level::{Level, LevelReading};
use rustkit_core::rows;
use rustkit_core::todos::{List, Todo};
use rustkit_store::{Store, Todo as StoreTodo};

/// The accelerometer arrives at 50 Hz and the screen refreshes at 60: the bridge
/// does not need to push every sample, so this is the interval the UI hears
/// about — roughly every other sample, about 30 a second.
///
/// 33 ms, because the Go side of this experiment picked that number first and the
/// two apps have to feel the same. Nothing in the arithmetic depends on it; the
/// only thing a different interval buys is a comparison that measures the wrong
/// difference.
const LEVEL_PUSH: Duration = Duration::from_millis(33);

/// Everything the app holds, in one place. Every field is the core's: the
/// bridge adds a clock and a path, and nothing else.
struct Kernel {
    list: List,
    level: Level,
    track: Track,
    foreground: bool,
    last_level_push: Instant,
    /// Turns the ticker off when the core stops.
    ticker: Arc<AtomicBool>,
    /// The app's own SQLite file — the list and the 20k benchmark rows. `None`
    /// when it could not be opened, which costs durability and nothing else.
    db: Option<Store>,
}

static KERNEL: Mutex<Option<Kernel>> = Mutex::new(None);
static LIFECYCLE: Mutex<()> = Mutex::new(());
static MUTATION: Mutex<()> = Mutex::new(());

/// Where events go: the C side registers a function at `listen()`, and clears
/// it when the runtime goes away.
static EMIT: Mutex<Option<extern "C" fn(*const c_char)>> = Mutex::new(None);

/// Runs `f` against the live kernel, if there is one.
///
/// A poisoned lock returns `None` instead of panicking: a panic that crosses the
/// C boundary aborts the app, and losing the list is a smaller failure than
/// losing the process.
fn with_kernel<R>(f: impl FnOnce(&mut Kernel) -> R) -> Option<R> {
    let mut slot = KERNEL.lock().ok()?;
    slot.as_mut().map(f)
}

/// Hands one event to the UI. The string is borrowed for the call only — the
/// C side copies it before queueing.
fn emit(json: String) {
    let Ok(text) = CString::new(json) else {
        return; // a NUL byte cannot cross this boundary
    };
    let sink = match EMIT.lock() {
        Ok(slot) => *slot,
        Err(_) => None,
    };
    if let Some(call) = sink {
        call(text.as_ptr());
    }
}

fn todos_event(todos: &[Todo]) -> String {
    serde_json::json!({ "kind": "todos", "todos": todos }).to_string()
}

/// The list, as the database crate stores it. `rustkit_store` must not depend on
/// the core (the core stays pure Rust and has no idea a database exists), so the
/// two shapes meet here — in the bridge, which is allowed to know both.
fn to_store(todos: &[Todo]) -> Vec<StoreTodo> {
    todos
        .iter()
        .map(|todo| StoreTodo {
            id: todo.id,
            title: todo.title.clone(),
            done: todo.done,
        })
        .collect()
}

fn from_store(todos: Vec<StoreTodo>) -> Vec<Todo> {
    todos
        .into_iter()
        .map(|todo| Todo {
            id: todo.id,
            title: todo.title,
            done: todo.done,
        })
        .collect()
}

/// Opens the core's own file and, once, imports the list the JSON years left
/// behind.
fn open_store(dir: &Path) -> Result<Store, String> {
    let store = Store::open(&dir.join("rustkit.db")).map_err(|err| err.to_string())?;
    import_legacy_list(&store, dir);
    Ok(store)
}

/// Moves an old `todos.json` into the database, exactly once.
///
/// Only into an empty table: someone who deleted their last row on purpose must
/// not get yesterday's list back on the next launch — that would be a
/// resurrection, not a migration. The file is renamed, not deleted, because it
/// is the only other copy and an import that went wrong should not be the end
/// of it. Every failure here is a note in the log, not a reason to refuse to
/// start: a list that cannot be imported is better than an app that cannot open.
fn import_legacy_list(store: &Store, dir: &Path) {
    if !matches!(store.count_todos(), Ok(0)) {
        return; // already has a list, or the table cannot be read
    }
    let legacy = dir.join("todos.json");
    let Ok(text) = fs::read_to_string(&legacy) else {
        return; // the normal case: no file from the JSON years
    };
    let Ok(todos) = serde_json::from_str::<Vec<Todo>>(&text) else {
        eprintln!(
            "rustkit: {} is not a list of todos; leaving it alone",
            legacy.display()
        );
        return;
    };
    if todos.is_empty() {
        return;
    }
    if let Err(err) = store.save_todos(&to_store(&todos)) {
        eprintln!("rustkit: could not import {}: {err}", legacy.display());
        return;
    }
    match fs::rename(&legacy, dir.join("todos.json.imported")) {
        Ok(()) => eprintln!("rustkit: imported {} todos from todos.json", todos.len()),
        Err(err) => eprintln!(
            "rustkit: imported {} todos, but todos.json could not be renamed ({err}) — fix that,\n         or the next launch will import them again over an empty list",
            todos.len()
        ),
    }
}

/// Holds the mutation lock through the save, then publishes only a durable list.
///
/// The rollback rule itself is `List::change_durably`'s (testable without a
/// database); this function only decides where "durable" is — the core's own
/// SQLite file, or nowhere at all when it could not be opened at start.
fn mutate_list(change: impl FnOnce(&mut List)) -> *mut c_char {
    let Ok(_mutation) = MUTATION.lock() else {
        eprintln!("rustkit: the mutation lock is poisoned; refusing a change");
        return out(json_error("the core is unavailable"));
    };
    let result = with_kernel(|kernel| {
        // Split the borrows: the list is written, the database is only read.
        let Kernel { list, db, .. } = kernel;
        let saved = list.change_durably(change, |todos| match db.as_ref() {
            Some(store) => store
                .save_todos(&to_store(todos))
                .map(|_| ())
                .map_err(|err| format!("could not save todos: {err}")),
            // No database: the list works for this session and says so in the
            // log at start. Pretending to fail here would make the app unusable
            // for a reason the UI could do nothing about.
            None => Ok(()),
        });
        match saved {
            Ok(()) => Ok(todos_event(list.snapshot())),
            Err(err) => Err(err),
        }
    });
    match result {
        Some(Ok(json)) => out(json),
        Some(Err(err)) => {
            // The UI shows this as well (the banner under the composer), but it
            // is the first thing to look for when someone says their list did
            // not survive — and the log is the copy that outlives the screen.
            eprintln!("rustkit: {err}");
            out(json_error(&err))
        }
        None => out(json_error("the core is not running")),
    }
}

/// The core's own clock, one line a second, and only while the app is in front
/// of the user: a background app pushing a line a second is a battery bug.
fn spawn_ticker(born: Instant) -> Arc<AtomicBool> {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    thread::spawn(move || {
        while !flag.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_secs(1));
            if flag.load(Ordering::Relaxed) {
                break;
            }
            let foreground = with_kernel(|k| k.foreground).unwrap_or(false);
            if foreground {
                let seconds = born.elapsed().as_secs();
                emit(
                    serde_json::json!({
                        "kind": "text",
                        "text": format!("up {seconds}s"),
                    })
                    .to_string(),
                );
            }
        }
    });
    stop
}

/// Hands a JSON string to the C side, which frees it with [`rustkit_free`].
fn out(json: String) -> *mut c_char {
    match CString::new(json) {
        Ok(text) => text.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

fn json_result<T: serde::Serialize>(key: &str, value: &T) -> String {
    serde_json::json!({ "ok": true, key: value }).to_string()
}

fn json_error(message: &str) -> String {
    serde_json::json!({ "ok": false, "error": message }).to_string()
}

/// Reads a borrowed descriptor from its first byte without moving its offset.
///
/// `Read for File` shares the descriptor's position with everyone else holding
/// the same one — and ArkTS hands this very descriptor to the image decoder a
/// line after it asks us what the file is, so a read that ends at EOF leaves the
/// decoder with nothing. `pread` (via [`FileExt::read_at`]) keeps the position
/// where the caller left it, and needs no `dup` to clean up afterwards.
///
/// [`FileExt::read_at`]: std::os::unix::fs::FileExt::read_at
struct FromStart<'a> {
    file: &'a std::fs::File,
    at: u64,
}

impl std::io::Read for FromStart<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let got = std::os::unix::fs::FileExt::read_at(self.file, buf, self.at)?;
        self.at += got as u64;
        Ok(got)
    }
}

/// Borrows a C string for the length of the call.
unsafe fn text_of(ptr: *const c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    // SAFETY: callers guarantee a live NUL-terminated pointer for this call.
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}

// ---------------------------------------------------------------- the exports
//
// `#[no_mangle] extern "C"`: what `napi_ohos.c` declares and calls. Nothing
// above this line knows these exist.

/// Registers where events go. `None` when the runtime is going away.
#[no_mangle]
pub extern "C" fn rustkit_set_emit(callback: Option<extern "C" fn(*const c_char)>) {
    if let Ok(mut slot) = EMIT.lock() {
        *slot = callback;
    }
}

/// Releases a string the core handed out. Every `char *` this file returns has
/// to come back here: Rust allocated it, so Rust frees it.
///
/// # Safety
///
/// `text` must be a pointer this library produced with [`out`], or null.
#[no_mangle]
pub unsafe extern "C" fn rustkit_free(text: *mut c_char) {
    if text.is_null() {
        return;
    }
    // SAFETY: the caller guarantees this pointer came from `out` exactly once.
    drop(unsafe { CString::from_raw(text) });
}

/// Starts the core: opens the database under `files_dir`, reads the list out of
/// it, starts the ticker.
///
/// # Safety
///
/// `files_dir` must be a NUL-terminated string, or null.
#[no_mangle]
pub unsafe extern "C" fn rustkit_start(files_dir: *const c_char) {
    let Ok(_lifecycle) = LIFECYCLE.lock() else {
        return;
    };
    let Ok(_mutation) = MUTATION.lock() else {
        return;
    };
    if KERNEL.lock().map(|slot| slot.is_some()).unwrap_or(true) {
        return;
    }
    // SAFETY: the caller guarantees a live NUL-terminated path for this call.
    let dir = unsafe { text_of(files_dir) };
    let dir = if dir.is_empty() {
        None
    } else {
        Some(PathBuf::from(&dir))
    };

    // The database is opened here and not on the first write: a list that cannot
    // be read is the one thing the UI would otherwise draw wrong, and finding out
    // at start means the log says so before the person starts typing.
    let (db, list) = match &dir {
        Some(dir) => match open_store(dir) {
            Ok(store) => match store.todos() {
                Ok(todos) => {
                    let list = List::load(from_store(todos));
                    eprintln!(
                        "rustkit: {} ({} todos)",
                        store.path().display(),
                        list.snapshot().len()
                    );
                    (Some(store), list)
                }
                Err(err) => {
                    eprintln!("rustkit: could not read the list back: {err}");
                    (Some(store), List::new())
                }
            },
            Err(err) => {
                // Same bargain as the Go version: no database is a session that
                // forgets, not an app that refuses to open.
                eprintln!("rustkit: no database, the list will not be saved: {err}");
                (None, List::new())
            }
        },
        None => {
            eprintln!("rustkit: started without a files dir, the list will not be saved");
            (None, List::new())
        }
    };

    let born = Instant::now();
    let ticker = spawn_ticker(born);

    let snapshot = todos_event(list.snapshot());
    if let Ok(mut slot) = KERNEL.lock() {
        *slot = Some(Kernel {
            list,
            level: Level::new(),
            track: Track::new(),
            foreground: true,
            last_level_push: Instant::now(),
            ticker,
            db,
        });
    }
    // Push the first list before anyone can ask for it: an event with nobody
    // listening is an event lost, and the UI subscribes before it starts us.
    emit(snapshot);
}

/// Stops the core and releases the database.
///
/// There is no save here on purpose: every change committed before the UI was
/// told about it (see [`mutate_list`]), so there is nothing left to write — and
/// a "save on the way out" would be a second path that only runs when the app
/// is being torn down, which is the worst time to discover it is wrong.
#[no_mangle]
pub extern "C" fn rustkit_stop() {
    let Ok(_lifecycle) = LIFECYCLE.lock() else {
        return;
    };
    let Ok(_mutation) = MUTATION.lock() else {
        return;
    };
    let taken = KERNEL.lock().ok().and_then(|mut slot| slot.take());
    if let Some(kernel) = taken {
        kernel.ticker.store(true, Ordering::Relaxed);
        eprintln!("rustkit: stopped ({} todos)", kernel.list.snapshot().len());
    }
}

#[no_mangle]
pub extern "C" fn rustkit_snapshot() -> *mut c_char {
    match with_kernel(|k| todos_event(k.list.snapshot())) {
        Some(json) => out(json),
        None => out(json_error("the core is not running")),
    }
}

/// Adds a row and answers with the list as it is now.
///
/// # Safety
///
/// `title` must be a NUL-terminated string, or null.
#[no_mangle]
pub unsafe extern "C" fn rustkit_add(title: *const c_char) -> *mut c_char {
    // SAFETY: the caller guarantees a live NUL-terminated title for this call.
    let title = unsafe { text_of(title) };
    mutate_list(|list| {
        list.add(&title);
    })
}

#[no_mangle]
pub extern "C" fn rustkit_toggle(id: i32) -> *mut c_char {
    mutate_list(|list| {
        list.toggle(i64::from(id));
    })
}

#[no_mangle]
pub extern "C" fn rustkit_remove(id: i32) -> *mut c_char {
    mutate_list(|list| {
        list.remove(i64::from(id));
    })
}

#[no_mangle]
pub extern "C" fn rustkit_remaining() -> i32 {
    with_kernel(|k| k.list.remaining() as i32).unwrap_or(0)
}

/// What a file the system opened is.
///
/// # Safety
///
/// `fd` must be open and owned by the caller: this borrows it, it neither closes
/// it nor moves its offset. `name` must be a NUL-terminated string, or null.
#[no_mangle]
pub unsafe extern "C" fn rustkit_inspect(fd: i32, name: *const c_char) -> *mut c_char {
    use std::mem::ManuallyDrop;
    use std::os::unix::io::FromRawFd;

    if fd < 0 {
        return out(json_error("no file descriptor"));
    }
    // SAFETY: the caller guarantees a live NUL-terminated name for this call.
    let name = unsafe { text_of(name) };
    // ManuallyDrop: the descriptor belongs to the UI, which closes it.
    // SAFETY: the caller guarantees `fd` is open; ManuallyDrop preserves its ownership.
    let file = ManuallyDrop::new(unsafe { std::fs::File::from_raw_fd(fd) });
    let mut reader = FromStart { file: &file, at: 0 };
    match file::inspect(&mut reader, &name) {
        Ok(info) => out(json_result("file", &info)),
        Err(err) => out(json_error(&err.to_string())),
    }
}

/// Describes decoded pixels.
///
/// # Safety
///
/// `data` must point at `length` readable bytes, and must stay alive for the
/// call: the buffer is borrowed, never kept.
#[no_mangle]
pub unsafe extern "C" fn rustkit_analyze_pixels(
    data: *const u8,
    length: usize,
    width: i32,
    height: i32,
    stride: i32,
) -> *mut c_char {
    if data.is_null() || length == 0 {
        return out(json_error("no pixels"));
    }
    // SAFETY: the caller guarantees `data` points to `length` readable bytes.
    let pixels = unsafe { std::slice::from_raw_parts(data, length) };
    match image::analyze_pixels(pixels, width, height, stride) {
        Ok(analysis) => out(json_result("image", &analysis)),
        Err(err) => out(json_error(&err.to_string())),
    }
}

/// One accelerometer sample. Called ~50 times a second: no allocation on the
/// way in, and the core decides when the UI hears about it.
#[no_mangle]
pub extern "C" fn rustkit_accel(x: f64, y: f64, z: f64) {
    let reading: Option<(LevelReading, bool)> = with_kernel(|k| {
        let reading = k.level.add(x, y, z);
        let due = k.last_level_push.elapsed() >= LEVEL_PUSH;
        if due {
            k.last_level_push = Instant::now();
        }
        (reading, due)
    });
    if let Some((reading, true)) = reading {
        emit(serde_json::json!({ "kind": "level", "level": reading }).to_string());
    }
}

#[no_mangle]
pub extern "C" fn rustkit_location(latitude: f64, longitude: f64, accuracy: f64) -> *mut c_char {
    let fix = GeoFix {
        lat: latitude,
        lon: longitude,
        accuracy,
    };
    match with_kernel(|k| k.track.fix(fix, Instant::now())) {
        Some(point) => out(json_result("point", &point)),
        None => out(json_error("the core is not running")),
    }
}

#[no_mangle]
pub extern "C" fn rustkit_location_reset() {
    with_kernel(|k| k.track.reset());
}

#[no_mangle]
pub extern "C" fn rustkit_lifecycle(foreground: i32) {
    with_kernel(|k| k.foreground = foreground != 0);
}

// ------------------------------------------------- the big list, and its wire
//
// The page comes back two ways on purpose: as JSON (what a browser would do) and
// in the compact form (no parse, fewer bytes). The UI measures both, so the same
// page has to be reachable by both.

#[no_mangle]
pub extern "C" fn rustkit_rows_count() -> i32 {
    rows::ROW_COUNT
}

/// A page as the compact form. The buffer is Rust's; the C side copies it into
/// an ArrayBuffer and hands it back through [`rustkit_rows_free`].
///
/// # Safety
///
/// `out_len` must be a valid pointer, or null.
#[no_mangle]
pub unsafe extern "C" fn rustkit_rows(offset: i32, limit: i32, out_len: *mut usize) -> *mut u8 {
    let bytes = rows::encode_rows(&rows::rows(offset, limit)).into_boxed_slice();
    if !out_len.is_null() {
        // SAFETY: the caller guarantees `out_len` is writable when non-null.
        unsafe { *out_len = bytes.len() };
    }
    Box::into_raw(bytes) as *mut u8
}

/// Releases a page [`rustkit_rows`] handed out.
///
/// # Safety
///
/// `ptr` must come from [`rustkit_rows`], with the `len` it reported.
#[no_mangle]
pub unsafe extern "C" fn rustkit_rows_free(ptr: *mut u8, len: usize) {
    if ptr.is_null() {
        return;
    }
    // SAFETY: the caller guarantees this pointer/length pair came from `rustkit_rows`.
    drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len)) });
}

#[no_mangle]
pub extern "C" fn rustkit_rows_json(offset: i32, limit: i32) -> *mut c_char {
    let page = rows::rows(offset, limit);
    out(serde_json::json!({ "rows": page }).to_string())
}

// ------------------------------------------------------------ the database
//
// The queries live in `rustkit-store`; this file only measures them and moves
// their answer across. `micros` is the store's own clock, so the UI can subtract
// it from the round trip and see what the bridge cost — the same split the Go
// version reports.

fn db_missing() -> String {
    json_error("no database: dbOpen was not called")
}

fn store_of(kernel: &mut Kernel) -> Option<&Store> {
    kernel.db.as_ref()
}

/// Opens (or creates) the core's own SQLite file under `files_dir`.
///
/// # Safety
///
/// `files_dir` must be a NUL-terminated string, or null.
#[no_mangle]
pub unsafe extern "C" fn rustkit_db_open(files_dir: *const c_char) -> *mut c_char {
    // SAFETY: the caller guarantees a live NUL-terminated path for this call.
    let dir = unsafe { text_of(files_dir) };
    if dir.is_empty() {
        return out(json_error("no files dir"));
    }
    let dir = PathBuf::from(&dir);
    // Idempotent, like the Go version: the UI may call this on every page show,
    // and `start` already opened the same file. A second open would be a second
    // handle on one file — the one thing this app is careful not to do.
    //
    // Opening here does not re-read the list: the memory copy is the one the UI
    // has been drawing, and pulling an older copy out from under it would make
    // the panel disagree with itself.
    let result = with_kernel(|kernel| -> Result<String, String> {
        if kernel.db.is_none() {
            kernel.db = Some(open_store(&dir)?);
        }
        Ok(kernel
            .db
            .as_ref()
            .map(|store| store.path().display().to_string())
            .unwrap_or_default())
    });
    match result {
        Some(Ok(path)) => out(serde_json::json!({ "ok": true, "path": path }).to_string()),
        Some(Err(error)) => out(serde_json::json!({
            "ok": false, "path": "", "error": error,
        })
        .to_string()),
        None => out(json_error("the core is not running")),
    }
}

#[no_mangle]
pub extern "C" fn rustkit_db_seed(count: i32) -> *mut c_char {
    let timed = with_kernel(|kernel| {
        store_of(kernel).map(|store| {
            let started = Instant::now();
            let result = store.seed(count);
            (result, started.elapsed().as_micros() as i64)
        })
    });
    match timed {
        Some(Some((result, micros))) => match result {
            Ok(rows) => out(serde_json::json!({
                "ok": true, "op": "seed", "rows": rows, "micros": micros,
            })
            .to_string()),
            Err(err) => out(serde_json::json!({
                "ok": false, "op": "seed", "rows": 0, "micros": micros, "error": err.to_string(),
            })
            .to_string()),
        },
        _ => out(db_missing()),
    }
}

#[no_mangle]
pub extern "C" fn rustkit_db_page(offset: i32, limit: i32) -> *mut c_char {
    let timed = with_kernel(|kernel| {
        store_of(kernel).map(|store| {
            let started = Instant::now();
            let result = store.page(offset, limit);
            (result, started.elapsed().as_micros() as i64)
        })
    });
    match timed {
        Some(Some((result, micros))) => match result {
            Ok(rows) => out(serde_json::json!({
                "ok": true, "rows": rows, "count": rows.len(), "micros": micros,
            })
            .to_string()),
            Err(err) => out(serde_json::json!({
                "ok": false, "rows": [], "count": 0, "micros": micros, "error": err.to_string(),
            })
            .to_string()),
        },
        _ => out(db_missing()),
    }
}

#[no_mangle]
pub extern "C" fn rustkit_db_count() -> i32 {
    with_kernel(|kernel| kernel.db.as_ref().and_then(|store| store.count().ok()))
        .flatten()
        .unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn rustkit_db_reset() -> *mut c_char {
    let timed = with_kernel(|kernel| {
        store_of(kernel).map(|store| {
            let started = Instant::now();
            let result = store.reset();
            (result, started.elapsed().as_micros() as i64)
        })
    });
    match timed {
        Some(Some((result, micros))) => match result {
            Ok(()) => out(serde_json::json!({
                "ok": true, "op": "reset", "rows": 0, "micros": micros,
            })
            .to_string()),
            Err(err) => out(serde_json::json!({
                "ok": false, "op": "reset", "rows": 0, "micros": micros, "error": err.to_string(),
            })
            .to_string()),
        },
        _ => out(db_missing()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // FFI tests share one process-global kernel; keep their lifecycle isolated.
    static TEST_KERNEL: Mutex<()> = Mutex::new(());

    /// The one kernel at a time, and a poisoned lock is still a lock: a test that
    /// panicked mid-session must report its own failure, not turn every test
    /// after it into `PoisonError` noise.
    fn exclusive() -> std::sync::MutexGuard<'static, ()> {
        TEST_KERNEL
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// A fresh directory under the system temp dir, named for the test.
    fn scratch(label: &str) -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("rustkit-{label}-{}-{nonce}", std::process::id()));
        fs::create_dir(&dir).unwrap();
        dir
    }

    /// Removes a directory and the extra files SQLite may leave beside the db.
    fn cleanup(dir: &Path) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(dir.join(format!("rustkit.db{suffix}")));
        }
        let _ = fs::remove_dir_all(dir);
    }

    fn start(dir: &Path) {
        let path = CString::new(dir.to_string_lossy().as_bytes()).unwrap();
        // SAFETY: `path` is alive and NUL-terminated for the call.
        unsafe { rustkit_start(path.as_ptr()) };
    }

    /// The list as the UI sees it, through the same call the UI makes.
    fn live() -> serde_json::Value {
        let snapshot = rustkit_snapshot();
        let parsed = serde_json::from_str(&unsafe { text_of(snapshot) }).unwrap();
        // SAFETY: `snapshot` came from `rustkit_snapshot` just above.
        unsafe { rustkit_free(snapshot) };
        parsed
    }

    fn titles(live: &serde_json::Value) -> Vec<String> {
        live["todos"]
            .as_array()
            .unwrap()
            .iter()
            .map(|todo| todo["title"].as_str().unwrap().to_owned())
            .collect()
    }

    #[test]
    fn a_string_made_here_comes_back_here() {
        let text = out("{\"ok\":true}".to_owned());
        assert!(!text.is_null());
        // SAFETY: `text` came from `out` just above.
        assert_eq!(unsafe { text_of(text) }, "{\"ok\":true}");
        unsafe { rustkit_free(text) };
    }

    #[test]
    fn inspecting_a_file_leaves_the_callers_offset_alone() {
        use std::io::{Seek, SeekFrom};
        use std::os::unix::io::AsRawFd;

        // ArkTS opens a photo, asks what it is, and hands the same descriptor to
        // the image decoder a line later. A read that walks the shared offset to
        // EOF would leave that decoder with an empty file — which is why this
        // reads from the start with pread instead of streaming the descriptor.
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("rustkit-inspect-{}-{nonce}", std::process::id()));
        fs::write(&path, b"abc").unwrap();
        let mut file = fs::File::open(&path).unwrap();
        file.seek(SeekFrom::Start(1)).unwrap();

        let name = CString::new("photo").unwrap();
        // SAFETY: the descriptor is open and owned by `file`, which outlives the call.
        let result = unsafe { rustkit_inspect(file.as_raw_fd(), name.as_ptr()) };
        let reply: serde_json::Value = serde_json::from_str(&unsafe { text_of(result) }).unwrap();
        assert_eq!(reply["ok"], true, "{reply}");
        assert_eq!(
            reply["file"]["bytes"], 3,
            "the whole file, not the rest of it"
        );
        assert_eq!(
            reply["file"]["sha256"],
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            file.stream_position().unwrap(),
            1,
            "the caller's offset moved: the decoder that reads this descriptor next would see EOF"
        );
        // SAFETY: `result` came from `rustkit_inspect` just above.
        unsafe { rustkit_free(result) };
        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn the_todos_event_is_the_shape_the_ui_reads() {
        let mut list = List::new();
        list.add("one");
        let json = todos_event(list.snapshot());
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["kind"], "todos");
        assert_eq!(parsed["todos"][0]["title"], "one");
        assert_eq!(parsed["todos"][0]["done"], false);
    }

    #[test]
    fn opening_the_database_before_start_returns_an_error() {
        let _guard = exclusive();
        let dir = CString::new(std::env::temp_dir().to_string_lossy().as_bytes()).unwrap();
        // SAFETY: `dir` remains alive and NUL-terminated for the call.
        let result = unsafe { rustkit_db_open(dir.as_ptr()) };
        let text = unsafe { text_of(result) };
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["ok"], false);
        assert_eq!(parsed["error"], "the core is not running");
        unsafe { rustkit_free(result) };
    }

    #[test]
    fn a_missing_directory_starts_without_persistence() {
        let _guard = exclusive();
        // The parent is deliberately absent: the database cannot be created there.
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let missing = std::env::temp_dir().join(format!(
            "rustkit-missing-parent-{}-{nonce}",
            std::process::id()
        ));
        let path = CString::new(missing.to_string_lossy().as_bytes()).unwrap();
        // SAFETY: the path stays NUL-terminated for the call.
        unsafe { rustkit_start(path.as_ptr()) };

        // No database is a session that forgets, not an app that refuses to open:
        // the list still works, and the log at start said durability was off.
        let title = CString::new("this session only").unwrap();
        // SAFETY: title stays NUL-terminated for this call.
        let result = unsafe { rustkit_add(title.as_ptr()) };
        let reply: serde_json::Value = serde_json::from_str(&unsafe { text_of(result) }).unwrap();
        assert_eq!(reply["kind"], "todos", "{reply}");
        assert_eq!(reply["todos"][0]["title"], "this session only");
        unsafe { rustkit_free(result) };
        rustkit_stop();

        // And the proof that it really did not save: start again, same path.
        unsafe { rustkit_start(path.as_ptr()) };
        assert!(
            titles(&live()).is_empty(),
            "nothing was written to a dir that does not exist"
        );
        rustkit_stop();
    }

    #[test]
    fn the_list_survives_a_restart() {
        let _guard = exclusive();
        let dir = scratch("restart");

        start(&dir);
        for title in ["kept", "second"] {
            let title = CString::new(title).unwrap();
            // SAFETY: title stays NUL-terminated for the call.
            let result = unsafe { rustkit_add(title.as_ptr()) };
            unsafe { rustkit_free(result) };
        }
        let toggled = rustkit_toggle(1); // id 1: the first row
                                         // SAFETY: `toggled` came from `rustkit_toggle` just above.
        unsafe { rustkit_free(toggled) };
        rustkit_stop();

        start(&dir);
        let after = live();
        assert_eq!(titles(&after), ["kept", "second"]);
        assert_eq!(
            after["todos"][0]["done"], true,
            "the done flag came back too"
        );

        // The id counter is rebuilt from the rows, so a restart does not skip a
        // number (the Go version's off-by-one, now pinned on this side too).
        let title = CString::new("third").unwrap();
        // SAFETY: title stays NUL-terminated for the call.
        let added = unsafe { rustkit_add(title.as_ptr()) };
        unsafe { rustkit_free(added) };
        assert_eq!(live()["todos"][2]["id"], 3);
        rustkit_stop();

        cleanup(&dir);
    }

    #[test]
    fn a_json_list_from_the_old_version_is_imported_once() {
        let _guard = exclusive();
        let dir = scratch("import");
        let legacy = vec![
            Todo {
                id: 7,
                title: "from the JSON years".to_owned(),
                done: false,
            },
            Todo {
                id: 8,
                title: "and a second one".to_owned(),
                done: true,
            },
        ];
        fs::write(dir.join("todos.json"), serde_json::to_vec(&legacy).unwrap()).unwrap();

        start(&dir);
        let after = live();
        assert_eq!(titles(&after), ["from the JSON years", "and a second one"]);
        assert_eq!(after["todos"][1]["done"], true);
        rustkit_stop();

        // Renamed, not deleted: the JSON is the only other copy, and the rename
        // is also what stops the next launch from importing it again.
        assert!(!dir.join("todos.json").exists(), "the old file moved away");
        assert!(dir.join("todos.json.imported").exists());

        // An emptied list stays empty across a restart. This is the resurrection
        // the import is written to avoid: an empty table is a legitimate state.
        start(&dir);
        for id in [7, 8] {
            let removed = rustkit_remove(id);
            // SAFETY: `removed` came from `rustkit_remove` just above.
            unsafe { rustkit_free(removed) };
        }
        assert!(titles(&live()).is_empty());
        rustkit_stop();
        start(&dir);
        assert!(titles(&live()).is_empty(), "the import must not run twice");
        rustkit_stop();

        cleanup(&dir);
    }

    #[test]
    fn starting_twice_does_not_replace_the_live_core() {
        let _guard = exclusive();
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "rustkit-bridge-test-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&dir).unwrap();
        let path = CString::new(dir.to_string_lossy().as_bytes()).unwrap();
        // SAFETY: `path` stays valid for both start calls.
        unsafe { rustkit_start(path.as_ptr()) };
        let title = CString::new("kept").unwrap();
        // SAFETY: `title` remains valid for the call.
        let added = unsafe { rustkit_add(title.as_ptr()) };
        unsafe { rustkit_free(added) };
        // A duplicate start must not replace state or spawn another ticker.
        unsafe { rustkit_start(path.as_ptr()) };
        let snapshot = rustkit_snapshot();
        let text = unsafe { text_of(snapshot) };
        assert!(text.contains("kept"));
        unsafe { rustkit_free(snapshot) };

        // Concurrent calls must serialize their writes, including the final disk state.
        let workers: Vec<_> = ["left", "right"]
            .into_iter()
            .map(|title| {
                thread::spawn(move || {
                    let title = CString::new(title).unwrap();
                    // SAFETY: title is live and NUL-terminated for the entire call.
                    let result = unsafe { rustkit_add(title.as_ptr()) };
                    unsafe { rustkit_free(result) };
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        rustkit_stop();

        // Read it back the way the app does after a restart: the rows and the
        // concurrent writers survive, and the file the UI's copy came from is the
        // database — not a JSON sidecar.
        start(&dir);
        let after = live();
        let titles = titles(&after);
        assert_eq!(titles.len(), 3, "{titles:?}");
        assert!(titles.contains(&"kept".to_owned()), "{titles:?}");
        assert!(titles.contains(&"left".to_owned()), "{titles:?}");
        assert!(titles.contains(&"right".to_owned()), "{titles:?}");
        rustkit_stop();
        cleanup(&dir);
    }
}
