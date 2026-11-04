// The Node-API side of the bridge: what ArkTS imports as 'librustcore.so'.
//
// Three things happen here:
//
//   - JavaScript calls into Rust (rustkit_snapshot, rustkit_add, …), on the main
//     thread, and gets a string of JSON back;
//   - Rust calls out to JavaScript, from its own thread: the string goes through
//     a threadsafe function, which is the only safe way to touch the JavaScript
//     engine from off the main thread;
//   - the module registers itself with the ArkTS runtime at load time.
//
// This file is the Go version's, symbol for symbol, with the prefix changed.
// That is not laziness — it is the claim being tested. If the seam is real, the
// glue cannot tell which language built the library underneath it.

// HarmonyOS's own headers. Without the NDK (a desktop's tooling, say) there is
// nothing here to build, so the file is empty and a stranger's analyzer is not
// shown functions it has no declarations for.
#if __has_include(<napi/native_api.h>) && __has_include(<hilog/log.h>)
#include <napi/native_api.h>
#include <hilog/log.h>

#include <stdlib.h>
#include <string.h>

// What Rust exports (see ../src/lib.rs). Strings come back malloc'd by Rust and
// go back through rustkit_free; the file descriptor is borrowed, never closed.
extern void rustkit_set_emit(void (*callback)(const char *));
extern void rustkit_free(char *text);
extern void rustkit_start(const char *files_dir);
extern void rustkit_stop(void);
extern char *rustkit_snapshot(void);
extern char *rustkit_add(const char *title);
extern char *rustkit_toggle(int32_t id);
extern char *rustkit_remove(int32_t id);
extern int32_t rustkit_remaining(void);
extern char *rustkit_inspect(int32_t fd, const char *name);
extern char *rustkit_analyze_pixels(const uint8_t *data, size_t length, int32_t width, int32_t height,
                                    int32_t stride);
extern void rustkit_accel(double x, double y, double z);
extern char *rustkit_location(double lat, double lon, double accuracy);
extern void rustkit_location_reset(void);
extern void rustkit_lifecycle(int32_t foreground);
extern int32_t rustkit_rows_count(void);
extern uint8_t *rustkit_rows(int32_t offset, int32_t limit, size_t *length);
extern void rustkit_rows_free(uint8_t *bytes, size_t length);
extern char *rustkit_rows_json(int32_t offset, int32_t limit);
extern char *rustkit_db_open(const char *files_dir);
extern char *rustkit_db_seed(int32_t n);
extern char *rustkit_db_page(int32_t offset, int32_t limit);
extern int32_t rustkit_db_count(void);
extern char *rustkit_db_reset(void);

#define RUSTKIT_LOG_DOMAIN 0x0
#define RUSTKIT_LOG_TAG "rustkit"

#define loge(...) OH_LOG_Print(LOG_APP, LOG_ERROR, RUSTKIT_LOG_DOMAIN, RUSTKIT_LOG_TAG, __VA_ARGS__)
#define logi(...) OH_LOG_Print(LOG_APP, LOG_INFO, RUSTKIT_LOG_DOMAIN, RUSTKIT_LOG_TAG, __VA_ARGS__)

// The function JavaScript gave to listen(), and the channel to it.
static napi_threadsafe_function emitFn;

// emitToJS is what Rust calls, from its own thread, with a string it owns for
// the length of the call only — so it is copied before it is queued.
static void emitToJS(const char *json) {
    if (!json) return;
    if (!emitFn) return; // the UI stopped listening
    char *copy = strdup(json);
    if (!copy) return;
    if (napi_call_threadsafe_function(emitFn, copy, napi_tsfn_nonblocking) != napi_ok) {
        free(copy);
    }
}

// emitCB runs on the main thread: JavaScript's turn.
static void emitCB(napi_env env, napi_value jsFn, void *ctx, void *data) {
    char *json = data;
    if (env == NULL || jsFn == NULL) { // the runtime is going away
        free(json);
        return;
    }
    napi_value arg, global, ret;
    napi_create_string_utf8(env, json, NAPI_AUTO_LENGTH, &arg);
    napi_get_undefined(env, &global);
    napi_call_function(env, global, jsFn, 1, &arg, &ret);
    free(json);
}

// toJS turns a string Rust made into a JavaScript string, and gives it back.
static napi_value toJS(napi_env env, char *json) {
    napi_value out;
    napi_create_string_utf8(env, json ? json : "", NAPI_AUTO_LENGTH, &out);
    if (json) rustkit_free(json);
    return out;
}

// toC copies a JavaScript string into a C string Rust can read.
static char *toC(napi_env env, napi_value v) {
    size_t len = 0;
    if (napi_get_value_string_utf8(env, v, NULL, 0, &len) != napi_ok) return NULL;
    char *s = malloc(len + 1);
    if (!s) return NULL;
    if (napi_get_value_string_utf8(env, v, s, len + 1, &len) != napi_ok) {
        free(s);
        return NULL;
    }
    return s;
}

// listen(fn): where the core's events go. Call it before start().
static napi_value Listen(napi_env env, napi_callback_info info) {
    size_t argc = 1;
    napi_value argv[1];
    if (napi_get_cb_info(env, info, &argc, argv, NULL, NULL) != napi_ok || argc < 1) {
        napi_throw_type_error(env, NULL, "listen(fn)");
        return NULL;
    }
    napi_valuetype t;
    napi_typeof(env, argv[0], &t);
    if (t != napi_function) {
        napi_throw_type_error(env, NULL, "listen(fn): fn must be a function");
        return NULL;
    }
    if (emitFn) {
        rustkit_set_emit(NULL);
        napi_release_threadsafe_function(emitFn, napi_tsfn_release);
        emitFn = NULL;
    }
    napi_value name;
    napi_create_string_utf8(env, "rustkit", NAPI_AUTO_LENGTH, &name);
    if (napi_create_threadsafe_function(env, argv[0], NULL, name, 0, 1, NULL, NULL, NULL, emitCB,
                                        &emitFn) != napi_ok) {
        loge("listen: no threadsafe function");
        emitFn = NULL;
        return NULL;
    }
    rustkit_set_emit(emitToJS);
    return NULL;
}

// start(filesDir): reads the list, starts the core's ticker.
static napi_value Start(napi_env env, napi_callback_info info) {
    size_t argc = 1;
    napi_value argv[1];
    char *dir = NULL;
    if (napi_get_cb_info(env, info, &argc, argv, NULL, NULL) == napi_ok && argc > 0) {
        dir = toC(env, argv[0]);
    }
    rustkit_start(dir);
    free(dir);
    logi("started");
    return NULL;
}

static napi_value Stop(napi_env env, napi_callback_info info) {
    (void)env;
    (void)info;
    rustkit_stop();
    return NULL;
}

static napi_value Snapshot(napi_env env, napi_callback_info info) {
    (void)info;
    return toJS(env, rustkit_snapshot());
}

// add(title): the list as it is after it.
static napi_value Add(napi_env env, napi_callback_info info) {
    size_t argc = 1;
    napi_value argv[1];
    if (napi_get_cb_info(env, info, &argc, argv, NULL, NULL) != napi_ok || argc < 1) {
        napi_throw_type_error(env, NULL, "add(title)");
        return NULL;
    }
    char *title = toC(env, argv[0]);
    if (!title) return NULL;
    napi_value out = toJS(env, rustkit_add(title));
    free(title);
    return out;
}

// id arg: the number the UI passes (a todo's).
static napi_value withID(napi_env env, napi_callback_info info, char *(*fn)(int32_t)) {
    size_t argc = 1;
    napi_value argv[1];
    int32_t id = 0;
    if (napi_get_cb_info(env, info, &argc, argv, NULL, NULL) != napi_ok || argc < 1) {
        napi_throw_type_error(env, NULL, "id expected");
        return NULL;
    }
    if (napi_get_value_int32(env, argv[0], &id) != napi_ok) return NULL;
    return toJS(env, fn(id));
}

static napi_value Toggle(napi_env env, napi_callback_info info) { return withID(env, info, rustkit_toggle); }

static napi_value Remove(napi_env env, napi_callback_info info) { return withID(env, info, rustkit_remove); }

static napi_value Remaining(napi_env env, napi_callback_info info) {
    (void)info;
    napi_value out;
    napi_create_int32(env, rustkit_remaining(), &out);
    return out;
}

// accel(x, y, z): one accelerometer sample (m/s²). Called from the sensor's
// callback, ~50 times a second: it does no allocation and no JavaScript, and the
// core decides when the UI hears about it.
static napi_value Accel(napi_env env, napi_callback_info info) {
    size_t argc = 3;
    napi_value argv[3];
    double x = 0, y = 0, z = 0;
    if (napi_get_cb_info(env, info, &argc, argv, NULL, NULL) != napi_ok || argc < 3) {
        napi_throw_type_error(env, NULL, "accel(x, y, z)");
        return NULL;
    }
    napi_get_value_double(env, argv[0], &x);
    napi_get_value_double(env, argv[1], &y);
    napi_get_value_double(env, argv[2], &z);
    rustkit_accel(x, y, z);
    return NULL;
}

// inspect(fd, name): what a file the system opened is, as JSON.
static napi_value Inspect(napi_env env, napi_callback_info info) {
    size_t argc = 2;
    napi_value argv[2];
    int32_t fd = -1;
    if (napi_get_cb_info(env, info, &argc, argv, NULL, NULL) != napi_ok || argc < 1) {
        napi_throw_type_error(env, NULL, "inspect(fd, name)");
        return NULL;
    }
    if (napi_get_value_int32(env, argv[0], &fd) != napi_ok) return NULL;
    char *name = argc > 1 ? toC(env, argv[1]) : NULL;
    char *json = rustkit_inspect(fd, name ? name : "file");
    napi_value out = toJS(env, json);
    free(name);
    return out;
}

// analyze(pixels, width, height[, stride]): what a decoded picture looks like.
// The ArrayBuffer is borrowed for the call — Rust never keeps the pointer.
static napi_value Analyze(napi_env env, napi_callback_info info) {
    size_t argc = 4;
    napi_value argv[4];
    if (napi_get_cb_info(env, info, &argc, argv, NULL, NULL) != napi_ok || argc < 3) {
        napi_throw_type_error(env, NULL, "analyze(pixels, width, height[, stride])");
        return NULL;
    }
    void *data = NULL;
    size_t length = 0;
    if (napi_get_arraybuffer_info(env, argv[0], &data, &length) != napi_ok) {
        napi_throw_type_error(env, NULL, "analyze: arg 0 must be an ArrayBuffer of RGBA pixels");
        return NULL;
    }
    int32_t width = 0, height = 0, stride = 0;
    if (napi_get_value_int32(env, argv[1], &width) != napi_ok) return NULL;
    if (napi_get_value_int32(env, argv[2], &height) != napi_ok) return NULL;
    if (argc > 3) napi_get_value_int32(env, argv[3], &stride);
    return toJS(env, rustkit_analyze_pixels(data, length, width, height, stride));
}

// location(lat, lon, accuracy): one fix in, the track so far out, as JSON.
static napi_value Location(napi_env env, napi_callback_info info) {
    size_t argc = 3;
    napi_value argv[3];
    double lat = 0, lon = 0, accuracy = 0;
    if (napi_get_cb_info(env, info, &argc, argv, NULL, NULL) != napi_ok || argc < 2) {
        napi_throw_type_error(env, NULL, "location(lat, lon, accuracy)");
        return NULL;
    }
    if (napi_get_value_double(env, argv[0], &lat) != napi_ok) return NULL;
    if (napi_get_value_double(env, argv[1], &lon) != napi_ok) return NULL;
    if (argc > 2) napi_get_value_double(env, argv[2], &accuracy);
    return toJS(env, rustkit_location(lat, lon, accuracy));
}

static napi_value LocationReset(napi_env env, napi_callback_info info) {
    (void)env;
    (void)info;
    rustkit_location_reset();
    return NULL;
}

// lifecycle(foreground): 1 on screen, 0 in the background.
static napi_value Lifecycle(napi_env env, napi_callback_info info) {
    size_t argc = 1;
    napi_value argv[1];
    int32_t fg = 1;
    if (napi_get_cb_info(env, info, &argc, argv, NULL, NULL) == napi_ok && argc > 0) {
        napi_get_value_int32(env, argv[0], &fg);
    }
    rustkit_lifecycle(fg);
    return NULL;
}

static napi_value RowsCount(napi_env env, napi_callback_info info) {
    (void)info;
    napi_value out;
    napi_create_int32(env, rustkit_rows_count(), &out);
    return out;
}

// rows(offset, limit): a page of the big list as an ArrayBuffer — one copy, no
// parsing, and shorter than the JSON of the same page.
static napi_value Rows(napi_env env, napi_callback_info info) {
    size_t argc = 2;
    napi_value argv[2];
    int32_t offset = 0, limit = 0;
    if (napi_get_cb_info(env, info, &argc, argv, NULL, NULL) != napi_ok || argc < 2) {
        napi_throw_type_error(env, NULL, "rows(offset, limit)");
        return NULL;
    }
    if (napi_get_value_int32(env, argv[0], &offset) != napi_ok) return NULL;
    if (napi_get_value_int32(env, argv[1], &limit) != napi_ok) return NULL;

    size_t length = 0;
    uint8_t *bytes = rustkit_rows(offset, limit, &length);
    napi_value out;
    void *data = NULL;
    if (napi_create_arraybuffer(env, length, &data, &out) != napi_ok) {
        rustkit_rows_free(bytes, length);
        return NULL;
    }
    if (bytes && length > 0) memcpy(data, bytes, length);
    rustkit_rows_free(bytes, length);
    return out;
}

// rowsJson(offset, limit): the same page as JSON, to measure against.
static napi_value RowsJSON(napi_env env, napi_callback_info info) {
    size_t argc = 2;
    napi_value argv[2];
    int32_t offset = 0, limit = 0;
    if (napi_get_cb_info(env, info, &argc, argv, NULL, NULL) != napi_ok || argc < 2) {
        napi_throw_type_error(env, NULL, "rowsJson(offset, limit)");
        return NULL;
    }
    napi_get_value_int32(env, argv[0], &offset);
    napi_get_value_int32(env, argv[1], &limit);
    return toJS(env, rustkit_rows_json(offset, limit));
}

// dbOpen(filesDir): not ported yet — it answers with the reason (CONTEXT.md).
static napi_value DbOpen(napi_env env, napi_callback_info info) {
    size_t argc = 1;
    napi_value argv[1];
    char *dir = NULL;
    if (napi_get_cb_info(env, info, &argc, argv, NULL, NULL) == napi_ok && argc > 0) {
        dir = toC(env, argv[0]);
    }
    char *json = rustkit_db_open(dir);
    free(dir);
    return toJS(env, json);
}

static napi_value DbSeed(napi_env env, napi_callback_info info) {
    size_t argc = 1;
    napi_value argv[1];
    int32_t n = 0;
    if (napi_get_cb_info(env, info, &argc, argv, NULL, NULL) != napi_ok || argc < 1) {
        napi_throw_type_error(env, NULL, "dbSeed(n)");
        return NULL;
    }
    napi_get_value_int32(env, argv[0], &n);
    return toJS(env, rustkit_db_seed(n));
}

static napi_value DbPage(napi_env env, napi_callback_info info) {
    size_t argc = 2;
    napi_value argv[2];
    int32_t offset = 0, limit = 0;
    if (napi_get_cb_info(env, info, &argc, argv, NULL, NULL) != napi_ok || argc < 2) {
        napi_throw_type_error(env, NULL, "dbPage(offset, limit)");
        return NULL;
    }
    if (napi_get_value_int32(env, argv[0], &offset) != napi_ok) return NULL;
    if (napi_get_value_int32(env, argv[1], &limit) != napi_ok) return NULL;
    return toJS(env, rustkit_db_page(offset, limit));
}

static napi_value DbCount(napi_env env, napi_callback_info info) {
    (void)info;
    napi_value out;
    napi_create_int32(env, rustkit_db_count(), &out);
    return out;
}

// dbReset(): empties the table, keeping the file.
static napi_value DbReset(napi_env env, napi_callback_info info) {
    (void)info;
    return toJS(env, rustkit_db_reset());
}

static napi_value Init(napi_env env, napi_value exports) {
    napi_property_descriptor desc[] = {
        {"listen", NULL, Listen, NULL, NULL, NULL, napi_default, NULL},
        {"start", NULL, Start, NULL, NULL, NULL, napi_default, NULL},
        {"stop", NULL, Stop, NULL, NULL, NULL, napi_default, NULL},
        {"snapshot", NULL, Snapshot, NULL, NULL, NULL, napi_default, NULL},
        {"add", NULL, Add, NULL, NULL, NULL, napi_default, NULL},
        {"toggle", NULL, Toggle, NULL, NULL, NULL, napi_default, NULL},
        {"remove", NULL, Remove, NULL, NULL, NULL, napi_default, NULL},
        {"remaining", NULL, Remaining, NULL, NULL, NULL, napi_default, NULL},
        {"inspect", NULL, Inspect, NULL, NULL, NULL, napi_default, NULL},
        {"analyze", NULL, Analyze, NULL, NULL, NULL, napi_default, NULL},
        {"accel", NULL, Accel, NULL, NULL, NULL, napi_default, NULL},
        {"location", NULL, Location, NULL, NULL, NULL, napi_default, NULL},
        {"locationReset", NULL, LocationReset, NULL, NULL, NULL, napi_default, NULL},
        {"lifecycle", NULL, Lifecycle, NULL, NULL, NULL, napi_default, NULL},
        {"rowsCount", NULL, RowsCount, NULL, NULL, NULL, napi_default, NULL},
        {"rows", NULL, Rows, NULL, NULL, NULL, napi_default, NULL},
        {"rowsJson", NULL, RowsJSON, NULL, NULL, NULL, napi_default, NULL},
        {"dbOpen", NULL, DbOpen, NULL, NULL, NULL, napi_default, NULL},
        {"dbSeed", NULL, DbSeed, NULL, NULL, NULL, napi_default, NULL},
        {"dbPage", NULL, DbPage, NULL, NULL, NULL, napi_default, NULL},
        {"dbCount", NULL, DbCount, NULL, NULL, NULL, napi_default, NULL},
        {"dbReset", NULL, DbReset, NULL, NULL, NULL, napi_default, NULL},
    };
    napi_define_properties(env, exports, sizeof desc / sizeof desc[0], desc);
    return exports;
}

static napi_module rustcoreModule = {
    1,           // nm_version
    0,           // nm_flags
    NULL,        // nm_filename
    Init,        // nm_register_func
    "rustcore",  // nm_modname: librustcore.so
    NULL,        // nm_priv
    {0},         // reserved
};

__attribute__((constructor)) static void rustcoreRegister(void) { napi_module_register(&rustcoreModule); }

#endif // __has_include(<napi/native_api.h>)
