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
fn save_list(path: &Path, todos: &[Todo]) -> Result<(), String> {
    let json = serde_json::to_vec(todos).map_err(|err| err.to_string())?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json).map_err(|err| err.to_string())?;
    fs::rename(&tmp, path).map_err(|err| err.to_string())
}

/// Holds the mutation lock through the save, then publishes only a durable list.
/// On disk failure, restore the exact in-memory state (including the next id).
fn mutate_list(change: impl FnOnce(&mut List)) -> *mut c_char {
    let Ok(_mutation) = MUTATION.lock() else {
        return out(json_error("the core is unavailable"));
    };
    match with_kernel(|kernel| {
        let before = kernel.list.clone();
        change(&mut kernel.list);
        if let Some(path) = &kernel.path {
            if let Err(err) = save_list(path, kernel.list.snapshot()) {
                kernel.list = before;
                return Err(format!("could not save todos: {err}"));
            }
        }
        Ok(todos_event(kernel.list.snapshot()))
    }) {
        Some(Ok(json)) => out(json),
        Some(Err(err)) => out(json_error(&err)),
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

/// Starts the core: reads the list from `files_dir`, starts its ticker.
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
    let Ok(_lifecycle) = LIFECYCLE.lock() else {
        return;
    };
    let Ok(_mutation) = MUTATION.lock() else {
        return;
    };
    let taken = KERNEL.lock().ok().and_then(|mut slot| slot.take());
    if let Some(kernel) = taken {
        kernel.ticker.store(true, Ordering::Relaxed);
        if let Some(path) = &kernel.path {
            if let Err(err) = save_list(path, kernel.list.snapshot()) {
                eprintln!("rustkit: could not save todos on stop: {err}");
            }
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
/// `fd` must be open and owned by the caller: this borrows it, it does not close
/// it. `name` must be a NUL-terminated string, or null.
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
    let mut file_ref = &*file;
    match file::inspect(&mut file_ref, &name) {
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
    let path = Path::new(&dir).join("rustkit.db");
    let result = with_kernel(|kernel| match Store::open(&path) {
        Ok(store) => {
            let where_it_is = store.path().display().to_string();
            drop(kernel.db.replace(store)); // a second open replaces and closes the first
            Ok(where_it_is)
        }
        Err(err) => Err(err.to_string()),
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

    #[test]
    fn a_string_made_here_comes_back_here() {
        let text = out("{\"ok\":true}".to_owned());
        assert!(!text.is_null());
        // SAFETY: `text` came from `out` just above.
        assert_eq!(unsafe { text_of(text) }, "{\"ok\":true}");
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

    #[test]
    fn opening_the_database_before_start_returns_an_error() {
        let _guard = TEST_KERNEL.lock().unwrap();
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
    fn failed_save_does_not_claim_success_or_change_the_live_list() {
        let _guard = TEST_KERNEL.lock().unwrap();
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        // The parent is deliberately absent: writing todos.json.tmp must fail.
        let missing = std::env::temp_dir().join(format!(
            "rustkit-missing-parent-{}-{nonce}",
            std::process::id()
        ));
        let dir = CString::new(missing.to_string_lossy().as_bytes()).unwrap();
        // SAFETY: the directory path stays NUL-terminated for this call.
        unsafe { rustkit_start(dir.as_ptr()) };
        let title = CString::new("must not appear").unwrap();
        // SAFETY: title stays NUL-terminated for this call.
        let result = unsafe { rustkit_add(title.as_ptr()) };
        let response: serde_json::Value =
            serde_json::from_str(&unsafe { text_of(result) }).unwrap();
        assert_eq!(response["ok"], false, "{response}");
        assert!(response["error"].is_string(), "{response}");
        unsafe { rustkit_free(result) };
        let snapshot = rustkit_snapshot();
        let live: serde_json::Value = serde_json::from_str(&unsafe { text_of(snapshot) }).unwrap();
        assert!(live["todos"].as_array().unwrap().is_empty(), "{live}");
        unsafe { rustkit_free(snapshot) };
        rustkit_stop();
    }

    #[test]
    fn failed_toggle_and_remove_keep_existing_todos_unchanged() {
        let _guard = TEST_KERNEL.lock().unwrap();
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "rustkit-blocked-save-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&dir).unwrap();
        let original = vec![Todo {
            id: 7,
            title: "existing".to_owned(),
            done: false,
        }];
        fs::write(
            dir.join("todos.json"),
            serde_json::to_vec(&original).unwrap(),
        )
        .unwrap();
        // Occupy the temporary-file path with a directory: writes fail reliably.
        fs::create_dir(dir.join("todos.json.tmp")).unwrap();
        let path = CString::new(dir.to_string_lossy().as_bytes()).unwrap();
        // SAFETY: path stays NUL-terminated for this call.
        unsafe { rustkit_start(path.as_ptr()) };
        for result in [rustkit_toggle(7), rustkit_remove(7)] {
            let reply: serde_json::Value =
                serde_json::from_str(&unsafe { text_of(result) }).unwrap();
            assert_eq!(reply["ok"], false, "{reply}");
            unsafe { rustkit_free(result) };
            let snapshot = rustkit_snapshot();
            let live: serde_json::Value =
                serde_json::from_str(&unsafe { text_of(snapshot) }).unwrap();
            assert_eq!(live["todos"][0]["id"], 7);
            assert_eq!(live["todos"][0]["done"], false);
            assert_eq!(live["todos"].as_array().unwrap().len(), 1);
            unsafe { rustkit_free(snapshot) };
        }
        rustkit_stop();
        let disk: Vec<Todo> =
            serde_json::from_slice(&fs::read(dir.join("todos.json")).unwrap()).unwrap();
        assert_eq!(disk, original);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn starting_twice_does_not_replace_the_live_core() {
        let _guard = TEST_KERNEL.lock().unwrap();
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
        let disk: Vec<Todo> =
            serde_json::from_slice(&fs::read(dir.join("todos.json")).unwrap()).unwrap();
        let titles: Vec<&str> = disk.iter().map(|todo| todo.title.as_str()).collect();
        assert_eq!(titles.len(), 3);
        assert!(titles.contains(&"kept"));
        assert!(titles.contains(&"left"));
        assert!(titles.contains(&"right"));
        fs::remove_dir_all(dir).unwrap();
    }
}
