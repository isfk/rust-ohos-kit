//! The app's database: a SQLite file this crate owns.
//!
//! It is **not** part of `rustkit-core`, and that separation is the whole point.
//! The core stays plain Rust so it can be tested without a phone; this crate is
//! where a C library enters, in exactly one place, the way `go/store` was kept
//! separate from `go/core` on the other side.
//!
//! The difference from Go is worth being honest about: Go had a pure-Go SQLite
//! (`modernc.org/sqlite`), so *both* of its packages stayed dependency-free and
//! `go test ./...` needed nothing but the toolchain. Here `rusqlite`'s `bundled`
//! feature compiles SQLite's C for the target — it crosses to
//! `aarch64-unknown-linux-ohos` fine (20 s, NDK clang), so this is a cost in
//! purity, not in capability.
//!
//! One file, one owner. This crate owns the file it opened; the platform's
//! `relationalStore` owns the one it manages. Pointing both at the same file is
//! `SQLITE_BUSY` waiting to happen, which is why the demo's comparison card keeps
//! them in two different files.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

/// One row of the table the demo keeps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Row {
    pub id: i32,
    pub title: String,
    pub done: bool,
}

/// What can go wrong. Small on purpose: the bridge turns it into a JSON string
/// and the panel shows it.
#[derive(Debug)]
pub enum StoreError {
    NoPath,
    Sql(rusqlite::Error),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoPath => f.write_str("store: no path"),
            Self::Sql(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Sql(err)
    }
}

type Result<T> = std::result::Result<T, StoreError>;

/// The app's list, as this crate stores it. It is not `rustkit_core::Todo`: the
/// two sides agree on a shape, not on a type, and the bridge does the mapping —
/// the same split `go/store` makes, for the same reason (`rust/store` must not
/// depend on the core, or the database would be able to bend the core's rules).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Todo {
    pub id: i64,
    pub title: String,
    pub done: bool,
}

/// Two tables in one file on purpose: `rows` is the 20k-row comparison, `todos`
/// is what the person typed. One file has one writer, and the core is that
/// writer — the platform's `relationalStore` owns its own file (see the module
/// docs), which is why the bench card keeps the two apart.
const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS rows (
	id    INTEGER PRIMARY KEY,
	title TEXT    NOT NULL,
	done  INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS todos (
	id    INTEGER PRIMARY KEY,
	title TEXT    NOT NULL,
	done  INTEGER NOT NULL
)";

/// One SQLite file. Its methods take `&self`: SQLite has one writer at a time
/// anyway, and the mutex says so instead of letting the pool turn it into
/// "database is locked" under load.
pub struct Store {
    conn: Mutex<Connection>,
    path: PathBuf,
}

impl Store {
    /// Opens the database at `path`, creating it if it isn't there. The path is
    /// the app's sandbox, handed over by ArkTS: this crate has no idea where a
    /// phone keeps its files.
    pub fn open(path: &Path) -> Result<Self> {
        if path.as_os_str().is_empty() {
            return Err(StoreError::NoPath);
        }
        let conn = Connection::open(path)?;
        for pragma in [
            // A reader never waits for the writer.
            "PRAGMA journal_mode=WAL",
            // A commit doesn't wait for the platter.
            "PRAGMA synchronous=NORMAL",
            // And if something else does hold the file, wait rather than fail.
            "PRAGMA busy_timeout=5000",
        ] {
            conn.execute_batch(pragma)?;
        }
        conn.execute_batch(SCHEMA)?;
        Ok(Self {
            conn: Mutex::new(conn),
            path: path.to_path_buf(),
        })
    }

    /// An in-memory database, for tests that only care about the queries.
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self {
            conn: Mutex::new(conn),
            path: PathBuf::from(":memory:"),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Replaces the table with `n` rows, in one transaction with one prepared
    /// statement: the way a bulk import is supposed to be written. The
    /// difference against one commit per row is the largest number in the Go
    /// repository's bench (9.6 µs vs 0.67 µs per row), and it has nothing to do
    /// with which language asks.
    pub fn seed(&self, n: i32) -> Result<i32> {
        if n < 0 {
            return Ok(0);
        }
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM rows", [])?;
        {
            let mut stmt = tx.prepare("INSERT INTO rows (id, title, done) VALUES (?1, ?2, ?3)")?;
            for i in 0..n {
                // The title is built here, inside the timed call, for both sides
                // of the comparison: the platform's writer builds its own too.
                stmt.execute((i, format!("row {i}"), i % 2 == 0))?;
            }
        }
        tx.commit()?;
        Ok(n)
    }

    /// How many rows the table holds.
    pub fn count(&self) -> Result<i32> {
        let conn = self.lock();
        Ok(conn.query_row("SELECT COUNT(*) FROM rows", [], |row| row.get(0))?)
    }

    /// Reads one screenful, in id order. An offset past the end is not an error:
    /// a list scrolled beyond its data is empty, not broken.
    pub fn page(&self, offset: i32, limit: i32) -> Result<Vec<Row>> {
        if offset < 0 || limit <= 0 {
            return Ok(Vec::new());
        }
        let conn = self.lock();
        let mut stmt =
            conn.prepare("SELECT id, title, done FROM rows ORDER BY id LIMIT ?1 OFFSET ?2")?;
        let rows = stmt
            .query_map((limit, offset), |row| {
                Ok(Row {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    done: row.get::<_, i64>(2)? != 0,
                })
            })?
            .collect::<std::result::Result<Vec<Row>, _>>()?;
        Ok(rows)
    }

    /// Empties the table, keeping the file.
    pub fn reset(&self) -> Result<()> {
        let conn = self.lock();
        conn.execute("DELETE FROM rows", [])?;
        Ok(())
    }

    // ----------------------------------------------------------- the app's list
    //
    // The list is not the benchmark: it is the only state a person typed, so it
    // is read once at start and written after every change. The whole list goes
    // in on each write — "who changed what" is the core's business, and a
    // delete-and-insert in one transaction keeps the two copies in step without
    // asking the UI to tell us about diffs it does not compute.

    /// The list, in id order — the order the UI draws it in.
    pub fn todos(&self) -> Result<Vec<Todo>> {
        let conn = self.lock();
        let mut stmt = conn.prepare("SELECT id, title, done FROM todos ORDER BY id")?;
        let todos = stmt
            .query_map([], |row| {
                Ok(Todo {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    done: row.get::<_, i64>(2)? != 0,
                })
            })?
            .collect::<std::result::Result<Vec<Todo>, _>>()?;
        Ok(todos)
    }

    /// How many rows the list holds.
    pub fn count_todos(&self) -> Result<i32> {
        let conn = self.lock();
        Ok(conn.query_row("SELECT COUNT(*) FROM todos", [], |row| row.get(0))?)
    }

    /// Replaces the list, in one transaction: a half-written list is never what
    /// the next launch reads.
    pub fn save_todos(&self, todos: &[Todo]) -> Result<usize> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM todos", [])?;
        {
            let mut stmt = tx.prepare("INSERT INTO todos (id, title, done) VALUES (?1, ?2, ?3)")?;
            for todo in todos {
                stmt.execute((todo.id, &todo.title, todo.done))?;
            }
        }
        tx.commit()?;
        Ok(todos.len())
    }

    /// A poisoned mutex is not worth taking the process down for: the caller gets
    /// an error and the panel says so.
    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seeded(n: i32) -> Store {
        let store = Store::in_memory().unwrap();
        store.seed(n).unwrap();
        store
    }

    #[test]
    fn seed_replaces_the_table() {
        let store = seeded(20);
        assert_eq!(store.count().unwrap(), 20);
        assert_eq!(store.seed(5).unwrap(), 5);
        assert_eq!(
            store.count().unwrap(),
            5,
            "seed replaces, it does not append"
        );
    }

    #[test]
    fn a_page_is_ordered_and_clipped() {
        let store = seeded(10);
        let page = store.page(0, 3).unwrap();
        assert_eq!(page.iter().map(|r| r.id).collect::<Vec<_>>(), [0, 1, 2]);
        assert_eq!(page[0].title, "row 0");
        assert!(page[0].done, "row 0 is even");
        assert!(!page[1].done);

        let tail = store.page(8, 10).unwrap();
        assert_eq!(tail.len(), 2, "a page past the end is clipped, not padded");
        assert!(store.page(50, 10).unwrap().is_empty());
        assert!(store.page(0, 0).unwrap().is_empty());
        assert!(store.page(-1, 5).unwrap().is_empty());
    }

    #[test]
    fn reset_keeps_the_file_and_empties_the_table() {
        let store = seeded(7);
        store.reset().unwrap();
        assert_eq!(store.count().unwrap(), 0);
        store.seed(2).unwrap();
        assert_eq!(store.count().unwrap(), 2, "the file is still usable");
    }

    #[test]
    fn the_file_is_where_it_was_asked_to_be() {
        // The path is the one thing this crate cannot invent: on a device it is
        // the app's sandbox directory, passed in from ArkTS.
        let path = std::env::temp_dir().join(format!("rustkit-store-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let store = Store::open(&path).unwrap();
            assert_eq!(store.path(), path.as_path());
            store.seed(3).unwrap();
            assert_eq!(store.count().unwrap(), 3);
        }
        let reopened = Store::open(&path).unwrap();
        assert_eq!(reopened.count().unwrap(), 3, "the rows survived a reopen");
        drop(reopened);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
        assert!(
            Store::open(Path::new("")).is_err(),
            "an empty path is an error"
        );
    }

    fn todos(ids: &[(i64, &str, bool)]) -> Vec<Todo> {
        ids.iter()
            .map(|(id, title, done)| Todo {
                id: *id,
                title: (*title).to_owned(),
                done: *done,
            })
            .collect()
    }

    #[test]
    fn the_list_round_trips_with_its_flags() {
        let store = Store::in_memory().unwrap();
        let list = todos(&[(1, "one", false), (2, "two", true)]);
        assert_eq!(store.save_todos(&list).unwrap(), 2);
        assert_eq!(store.count_todos().unwrap(), 2);
        assert_eq!(
            store.todos().unwrap(),
            list,
            "the id, the title and the done flag all come back"
        );
    }

    #[test]
    fn saving_replaces_the_list_instead_of_appending() {
        // Every change arrives as the whole list (see `List::change_durably`), so
        // a delete-then-insert has to be what "save" means: appending would make
        // the list grow on every keystroke that reached the disk twice.
        let store = Store::in_memory().unwrap();
        store.save_todos(&todos(&[(1, "one", false)])).unwrap();
        store.save_todos(&todos(&[(7, "seven", true)])).unwrap();
        assert_eq!(store.todos().unwrap(), todos(&[(7, "seven", true)]));

        // And an empty list is a real state, not a no-op: that is what the UI
        // shows after the last row is deleted, and a restart has to agree.
        store.save_todos(&[]).unwrap();
        assert_eq!(store.count_todos().unwrap(), 0);
    }

    #[test]
    fn the_two_tables_share_one_file_without_meeting() {
        let store = Store::in_memory().unwrap();
        store.seed(5).unwrap();
        store.save_todos(&todos(&[(3, "three", false)])).unwrap();
        assert_eq!(store.count().unwrap(), 5, "the bench rows are untouched");
        assert_eq!(store.count_todos().unwrap(), 1);
        store.reset().unwrap();
        assert_eq!(
            store.count_todos().unwrap(),
            1,
            "reset is the rows table only"
        );
    }
}
