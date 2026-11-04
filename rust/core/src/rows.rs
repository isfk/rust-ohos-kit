//! The big list, and the two ways of getting a page of it over the bridge.
//!
//! The app draws 2000 rows; the comparison is between what that page costs as
//! JSON and what it costs in a form the UI can read without parsing. Nothing
//! here needs a phone: a page is a page, whichever side asks for it.

use serde::{Deserialize, Serialize};

/// How many rows the demo's big list has: enough that the wire format matters,
/// small enough to stay honest on a phone.
pub const ROW_COUNT: i32 = 20_000;

/// One row of the big list. The same shape as a todo, on purpose: the UI can
/// reuse its rendering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Row {
    pub id: i32,
    pub title: String,
    pub done: bool,
}

/// A page of the big list, made on demand: a page of 200 costs what 200 rows
/// cost, whether the list holds 20 or 20,000.
pub fn rows(offset: i32, limit: i32) -> Vec<Row> {
    let offset = offset.max(0);
    if limit <= 0 || offset >= ROW_COUNT {
        return Vec::new();
    }
    let limit = limit.min(ROW_COUNT - offset);
    (0..limit)
        .map(|i| {
            let n = offset + i;
            Row {
                id: n + 1,
                title: format!(
                    "Row {} · the quick brown fox jumps over the lazy dog",
                    n + 1
                ),
                done: n % 7 == 0,
            }
        })
        .collect()
}

/// The compact form the UI reads: `u32` count, then per row `u32` id, `u8` done,
/// `u16` title length, title bytes.
///
/// The saving is the syntax, not the content: 7 bytes of framing per row against
/// roughly 16 in JSON, and no parse at all. For the titles below that lands at
/// about 30% fewer bytes (the app measured 173 KB JSON against 120 KB compact for
/// a 2000-row page) — the words themselves are most of both.
pub fn encode_rows(rows: &[Row]) -> Vec<u8> {
    let size = 4 + rows
        .iter()
        .map(|r| 4 + 1 + 2 + r.title.len())
        .sum::<usize>();
    let mut out = Vec::with_capacity(size);
    out.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    for row in rows {
        out.extend_from_slice(&(row.id as u32).to_le_bytes());
        out.push(u8::from(row.done));
        out.extend_from_slice(&(row.title.len() as u16).to_le_bytes());
        out.extend_from_slice(row.title.as_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads the compact form back, the way `RustCore.ts` does.
    fn decode(buf: &[u8]) -> Vec<Row> {
        let mut out = Vec::new();
        let count = u32::from_le_bytes(buf[0..4].try_into().unwrap()) as usize;
        let mut at = 4;
        for _ in 0..count {
            let id = u32::from_le_bytes(buf[at..at + 4].try_into().unwrap()) as i32;
            at += 4;
            let done = buf[at] == 1;
            at += 1;
            let len = u16::from_le_bytes(buf[at..at + 2].try_into().unwrap()) as usize;
            at += 2;
            let title = String::from_utf8(buf[at..at + len].to_vec()).unwrap();
            at += len;
            out.push(Row { id, title, done });
        }
        assert_eq!(at, buf.len(), "the buffer has bytes nobody claimed");
        out
    }

    #[test]
    fn a_page_is_a_slice_of_the_list() {
        let page = rows(0, 3);
        assert_eq!(page.len(), 3);
        assert_eq!(page[0].id, 1);
        assert!(page[0].done); // 0 % 7
        assert_eq!(page[1].id, 2);
        assert!(!page[1].done);
        assert!(page[0].title.starts_with("Row 1 · the quick brown fox"));

        let last = rows(19_990, 20);
        assert_eq!(last.len(), 10, "a page past the end is clipped, not padded");
        assert_eq!(last[9].id, 20_000);
    }

    #[test]
    fn silly_arguments_get_an_empty_page() {
        assert!(rows(0, 0).is_empty());
        assert!(rows(0, -5).is_empty());
        assert!(rows(ROW_COUNT, 10).is_empty());
        assert!(rows(-10, 2).len() == 2, "a negative offset is 0, not empty");
    }

    #[test]
    fn the_compact_form_decodes_back() {
        let page = rows(0, 2000);
        let encoded = encode_rows(&page);
        assert_eq!(decode(&encoded), page);

        // What the comparison is about: the same page, two sizes. Not a tenth —
        // for these titles it is about two thirds, because the title is most of
        // both encodings and only the framing differs.
        let json = serde_json::to_vec(&page).unwrap();
        assert!(
            encoded.len() * 4 < json.len() * 3,
            "compact {} vs json {} (want under 75%)",
            encoded.len(),
            json.len()
        );
    }

    #[test]
    fn every_seventh_row_is_done() {
        let page = rows(0, 14);
        let done: Vec<bool> = page.iter().map(|r| r.done).collect();
        assert_eq!(
            done.iter().filter(|d| **d).count(),
            2,
            "two of fourteen: rows 1 and 8"
        );
        assert!(done[0] && done[7]);
    }
}
