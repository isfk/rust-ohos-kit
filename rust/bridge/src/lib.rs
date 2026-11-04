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
//! What is *not* here, and why, is in `CONTEXT.md`: the wire format and the
//! SQLite file are the next two steps, and the database is the one place where
//! Rust costs more than Go did (`rusqlite`'s bundled SQLite wants a C compiler
//! for the target; there is no `modernc.org/sqlite` on this side).

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
use rustkit_store::Store;

/// The accelerometer arrives at 50 Hz and the screen refreshes at 60: the
/// bridge does not need to push every sample, so this is the interval the UI
/// hears about.
const LEVEL_PUSH: Duration = Duration::from_millis(50);

/// Everything the app holds, in one place. Every field is the core's: the
/// bridge adds a clock and a path, and nothing else.
struct Kernel {
    list: List,
    /// Where the list is written. `None` until `start`, which is what the first
    /// launch looks like.
    path: Option<PathBuf>,
    level: Level,
    track: Track,
    foreground: bool,
    last_level_push: Instant,
    born: Instant,
    /// Turns the ticker off when the core stops.
    ticker: Arc<AtomicBool>,
    /// The core's own SQLite file, once `dbOpen` has opened one.
    db: Option<Store>,
}

static KERNEL: Mutex<Option<Kernel>> = Mutex::new(None);

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

/// Reads the list from disk. A missing file is an empty list, not an error: a
/// first launch is not a failure.
fn load_list(path: &Path) -> List {
    match fs::read_to_string(path) {
        Ok(text) => List::load(serde_json::from_str::<Vec<Todo>>(&text).unwrap_or_default()),
        Err(_) => List::new(),
    }
}

/// Writes the whole list through a temporary file, so a half-written list is
/// never what a reader finds.
fn save_list(path: &Path, todos: &[Todo]) {
    let Ok(json) = serde_json::to_string(todos) else {
        return;
    };
    let tmp = path.with_extension("json.tmp");
    if fs::write(&tmp, json).is_ok() {
        let _ = fs::rename(&tmp, path);
    }
}

/// The core's own clock, one line a second, and only while the app is in front
/// of the user: a background app pushing a line a second is a battery bug.
fn spawn_ticker(born: Instant) -> Arc<AtomicBool> {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    thread::spawn(move || {
        while !flag.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(500));
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

/// Borrows a C string for the length of the call.
fn text_of(ptr: *const c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    // SAFETY: the pointer comes from `toC` on the C side, which NUL-terminates.
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
    drop(CString::from_raw(text));
}

/// Starts the core: reads the list from `files_dir`, starts its ticker.
///
/// # Safety
///
/// `files_dir` must be a NUL-terminated string, or null.
#[no_mangle]
pub unsafe extern "C" fn rustkit_start(files_dir: *const c_char) {
    let dir = text_of(files_dir);
    let path = if dir.is_empty() {
        None
    } else {
        Some(Path::new(&dir).join("todos.json"))
    };
    let list = match &path {
        Some(p) => load_list(p),
        None => List::new(),
    };
    let born = Instant::now();
    let ticker = spawn_ticker(born);

    let snapshot = todos_event(list.snapshot());
    if let Ok(mut slot) = KERNEL.lock() {
        *slot = Some(Kernel {
            list,
            path,
            level: Level::new(),
            track: Track::new(),
            foreground: true,
            last_level_push: Instant::now(),
            born,
            ticker,
            db: None,
        });
    }
    // Push the first list before anyone can ask for it: an event with nobody
    // listening is an event lost, and the UI subscribes before it starts us.
    emit(snapshot);
}

#[no_mangle]
pub extern "C" fn rustkit_stop() {
    let taken = KERNEL.lock().ok().and_then(|mut slot| slot.take());
    if let Some(kernel) = taken {
        kernel.ticker.store(true, Ordering::Relaxed);
        if let Some(path) = &kernel.path {
            save_list(path, kernel.list.snapshot());
        }
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
    let title = text_of(title);
    let saved = with_kernel(|k| {
        k.list.add(&title);
        let snapshot = k.list.snapshot().to_vec();
        (snapshot, k.path.clone())
    });
    match saved {
        Some((todos, path)) => {
            if let Some(path) = path {
                save_list(&path, &todos);
            }
            out(todos_event(&todos))
        }
        None => out(json_error("the core is not running")),
    }
}

#[no_mangle]
pub extern "C" fn rustkit_toggle(id: i32) -> *mut c_char {
    let saved = with_kernel(|k| {
        k.list.toggle(i64::from(id));
        (k.list.snapshot().to_vec(), k.path.clone())
    });
    match saved {
        Some((todos, path)) => {
            if let Some(path) = path {
                save_list(&path, &todos);
            }
            out(todos_event(&todos))
        }
        None => out(json_error("the core is not running")),
    }
}

#[no_mangle]
pub extern "C" fn rustkit_remove(id: i32) -> *mut c_char {
    let saved = with_kernel(|k| {
        k.list.remove(i64::from(id));
        (k.list.snapshot().to_vec(), k.path.clone())
    });
    match saved {
        Some((todos, path)) => {
            if let Some(path) = path {
                save_list(&path, &todos);
            }
            out(todos_event(&todos))
        }
        None => out(json_error("the core is not running")),
    }
}

#[no_mangle]
pub extern "C" fn rustkit_remaining() -> i32 {
    with_kernel(|k| k.list.remaining() as i32).unwrap_or(0)
}

/// What a file the system opened is.
///
/// # Safety
///
/// `fd` must be open and owned by the caller: this borrows it, it does not close
/// it. `name` must be a NUL-terminated string, or null.
#[no_mangle]
pub unsafe extern "C" fn rustkit_inspect(fd: i32, name: *const c_char) -> *mut c_char {
    use std::mem::ManuallyDrop;
    use std::os::unix::io::FromRawFd;

    if fd < 0 {
        return out(json_error("no file descriptor"));
    }
    let name = text_of(name);
    // ManuallyDrop: the descriptor belongs to the UI, which closes it.
    let file = ManuallyDrop::new(std::fs::File::from_raw_fd(fd));
    match file::inspect(&mut (&*file), &name) {
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
    let pixels = std::slice::from_raw_parts(data, length);
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
    with_kernel(|k| {
        k.foreground = foreground != 0;
        k.born
    });
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
        *out_len = bytes.len();
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
    drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len)));
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
    let dir = text_of(files_dir);
    if dir.is_empty() {
        return out(json_error("no files dir"));
    }
    let path = Path::new(&dir).join("rustkit.db");
    match Store::open(&path) {
        Ok(store) => {
            let where_it_is = store.path().display().to_string();
            if let Some(inserted) = with_kernel(|k| k.db.replace(store)) {
                drop(inserted); // a second open replaces the first, and closes it
            }
            out(serde_json::json!({ "ok": true, "path": where_it_is }).to_string())
        }
        Err(err) => out(serde_json::json!({
            "ok": false, "path": "", "error": err.to_string(),
        })
        .to_string()),
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

    #[test]
    fn a_string_made_here_comes_back_here() {
        let text = out("{\"ok\":true}".to_owned());
        assert!(!text.is_null());
        // SAFETY: `text` came from `out` just above.
        assert_eq!(text_of(text), "{\"ok\":true}");
        unsafe { rustkit_free(text) };
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
}
