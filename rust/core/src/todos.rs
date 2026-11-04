//! The list: the only state a person typed, and the only state that survives a
//! restart.

use serde::{Deserialize, Serialize};

/// One row of the list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Todo {
    pub id: i64,
    pub title: String,
    pub done: bool,
}

/// The list, and the rule for its ids.
///
/// The id counter lives here and not in the caller because "never reuse an id"
/// is a property of the list, not of the button that adds a row.
#[derive(Debug, Default)]
pub struct List {
    todos: Vec<Todo>,
    next: i64,
}

impl List {
    pub fn new() -> Self {
        Self::default()
    }

    /// Rebuilds a list that came back from disk.
    ///
    /// `next` is the highest id on disk, not one past it: [`List::add`]
    /// increments before it uses the number. The Go version of this file set it
    /// to max+1 against an add that pre-incremented, so every restart skipped an
    /// id — the kind of off-by-one a test has to pin, because nothing else
    /// notices.
    ///
    /// The flip side, on purpose: if the last row was deleted, the counter comes
    /// back lower than it was, and that id can be handed out again. Nothing holds
    /// an id across a restart — the UI redraws from this list — so reuse is
    /// harmless, and a separate counter on disk would be one more thing to keep
    /// in step with the rows.
    pub fn load(todos: Vec<Todo>) -> Self {
        let next = todos.iter().map(|t| t.id).max().unwrap_or(0);
        Self { todos, next }
    }

    pub fn snapshot(&self) -> &[Todo] {
        &self.todos
    }

    /// How many rows are still open. The UI shows it, so the list answers it.
    pub fn remaining(&self) -> usize {
        self.todos.iter().filter(|t| !t.done).count()
    }

    /// Adds a row and returns the list as it is now, which is what the UI draws.
    /// A blank title is not a row: the rule belongs here, not in the text field.
    pub fn add(&mut self, title: &str) -> &[Todo] {
        let title = title.trim();
        if !title.is_empty() {
            self.next += 1;
            self.todos.push(Todo {
                id: self.next,
                title: title.to_owned(),
                done: false,
            });
        }
        &self.todos
    }

    pub fn toggle(&mut self, id: i64) -> &[Todo] {
        if let Some(todo) = self.todos.iter_mut().find(|t| t.id == id) {
            todo.done = !todo.done;
        }
        &self.todos
    }

    pub fn remove(&mut self, id: i64) -> &[Todo] {
        self.todos.retain(|t| t.id != id);
        &self.todos
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titles(list: &List) -> Vec<String> {
        list.snapshot().iter().map(|t| t.title.clone()).collect()
    }

    #[test]
    fn adds_and_counts() {
        let mut list = List::new();
        list.add("  read the profile  ");
        list.add("ship the dark theme");
        assert_eq!(titles(&list), ["read the profile", "ship the dark theme"]);
        assert_eq!(list.remaining(), 2);

        let id = list.snapshot()[0].id;
        list.toggle(id);
        assert_eq!(list.remaining(), 1);
        list.remove(id);
        assert_eq!(list.remaining(), 1);
        assert_eq!(titles(&list), ["ship the dark theme"]);
    }

    #[test]
    fn a_blank_title_is_not_a_row() {
        let mut list = List::new();
        list.add("");
        list.add("   ");
        assert!(list.snapshot().is_empty());
    }

    #[test]
    fn ids_do_not_skip_across_a_reload() {
        // The bug the Go version shipped and then fixed: load() set the counter
        // to max+1 while add() incremented first, so the first row after every
        // restart jumped a number.
        let mut list = List::new();
        list.add("one");
        list.add("two");

        let mut reloaded = List::load(list.snapshot().to_vec());
        reloaded.add("three");
        assert_eq!(reloaded.snapshot().last().unwrap().id, 3);
    }

    #[test]
    fn an_id_freed_before_a_reload_can_come_back() {
        // Documented consequence of keeping the counter in the rows rather than
        // beside them: nothing holds an id across a restart, so reuse is fine —
        // but it should be a decision, not a surprise.
        let mut list = List::new();
        list.add("one");
        list.add("two");
        let removed = list.snapshot()[1].id;
        list.remove(removed);

        let mut reloaded = List::load(list.snapshot().to_vec());
        reloaded.add("three");
        assert_eq!(reloaded.snapshot().last().unwrap().id, removed);
        assert_eq!(reloaded.snapshot().len(), 2);
    }

    #[test]
    fn toggling_an_id_that_is_gone_is_not_an_error() {
        let mut list = List::new();
        list.add("one");
        assert_eq!(list.toggle(99).len(), 1);
        assert_eq!(list.remaining(), 1);
    }
}
