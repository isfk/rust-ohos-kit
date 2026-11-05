// The app's output, in hilog.
//
// HarmonyOS gives a process's fds 1 and 2 to nobody, so what the core prints —
// and what the Rust runtime writes on its way out of a panic — is lost. Here
// those two descriptors go into a pipe that a thread of their own reads into
// hilog, tagged `rustkit`:
//
//   hdc shell hilog -T rustkit
//
// This is the Go version's `go/log_linux.c`, file for file: the core says what
// happened with `eprintln!`, and this is the part that knows a phone has no
// console. It also keeps `rust/bridge` free of any C symbol *calls* — Rust still
// references nothing in C, which is what the `build.rs` comment is about.
//
// A C thread and not a Rust one, for the reason the Go version gives: the
// process exits right after a panic writes its message, and only a thread
// already blocked in read(2) gets there first.
//
// `hilog/log.h` is the NDK's, so it is only there when the target is the device.
// `build.rs` is what knows that (it defines RUSTKIT_WITH_HILOG for an OHOS
// target), and this file compiles to nothing without it — which is also what
// keeps a desktop's analyzer from reporting a header it cannot see.

#if defined(RUSTKIT_WITH_HILOG)

#include <hilog/log.h>

#include <errno.h>
#include <pthread.h>
#include <stdint.h>
#include <unistd.h>

#define RUSTKIT_LOG_DOMAIN 0x0
#define RUSTKIT_LOG_TAG "rustkit"

static void *rustkitLogReader(void *arg) {
    int fd = (int)(intptr_t)arg;
    char buf[1024];
    for (;;) {
        ssize_t n = read(fd, buf, sizeof buf - 1);
        if (n < 0) {
            if (errno == EINTR) continue;
            break;
        }
        if (n == 0) break;
        while (n > 0 && (buf[n - 1] == '\n' || buf[n - 1] == '\r')) n--;
        buf[n] = 0;
        if (n == 0) continue;
        // One line per write: "%{public}s" is what hilog needs to print a string
        // instead of redacting it.
        OH_LOG_Print(LOG_APP, LOG_WARN, RUSTKIT_LOG_DOMAIN, RUSTKIT_LOG_TAG, "%{public}s", buf);
    }
    return NULL;
}

// Priority 101 is the earliest an ELF constructor may ask for, and this wants to
// be first: whatever the core prints while *starting* is the most interesting
// output there is, and napi_ohos.c registers the module in a constructor of its
// own.
__attribute__((constructor(101))) static void rustkitLogStart(void) {
    int fds[2];
    if (pipe(fds) != 0) return;
    if (dup2(fds[1], 1) < 0 || dup2(fds[1], 2) < 0) {
        close(fds[0]);
        close(fds[1]);
        return;
    }
    close(fds[1]);
    pthread_t t;
    if (pthread_create(&t, NULL, rustkitLogReader, (void *)(intptr_t)fds[0]) == 0) {
        pthread_detach(t);
    } else {
        close(fds[0]);
    }
}

#endif // defined(RUSTKIT_WITH_HILOG)
