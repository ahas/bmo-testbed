//! The ceiling every verb answers under (docs/boundary.md, 「The binary」): an answer cut to whole items,
//! counted in characters, with one line saying what was left out.

use bmo_testbed_core::answer::{Cut, cap_items, count_text};

fn lines(kept: &[String]) -> String {
  kept.join("\n")
}

fn many(item: &str, copies: usize) -> Vec<String> {
  vec![item.to_owned(); copies]
}

fn cut(limit: usize) -> Cut<'static> {
  Cut {
    unit: "nodes",
    how: "ask with narrower words.",
    limit,
  }
}

// "a list that fits comes back whole, and so does an empty one"
#[test]
fn a_list_that_fits_comes_back_whole() {
  let items = many("one", 3);
  let capped = cap_items(&items, lines, Cut::new("nodes", "ask with narrower words."));

  assert_eq!((capped.kept.len(), capped.left, capped.note), (3, 0, None));
  assert_eq!(cap_items(&[] as &[String], lines, cut(10)).note, None);
}

// "a list over the ceiling is cut to whole items, counted in characters, not bytes"
#[test]
fn a_list_over_the_ceiling_is_cut_to_whole_items_counted_in_characters() {
  let items = many(&"\u{e9}".repeat(9), 40);
  let capped = cap_items(&items, lines, cut(100));

  assert_eq!((capped.kept.len(), capped.left), (10, 30));
  assert_eq!(
    capped.note.as_deref(),
    Some("-- cut at the 100-character ceiling: 10 of 40 nodes. For the other 30: ask with narrower words.")
  );

  let thousands = many(&"x".repeat(9), 4000);
  let grouped = cap_items(&thousands, lines, cut(100));

  assert_eq!(
    grouped.note.as_deref(),
    Some(
      "-- cut at the 100-character ceiling: 10 of 4,000 nodes. For the other 3,990: ask with narrower words."
    )
  );
}

// "where one item alone is over the ceiling, nothing comes back but the line"
#[test]
fn one_item_alone_over_the_ceiling_gives_nothing_but_the_line() {
  let items = vec!["x".repeat(101), "y".to_owned()];
  let capped = cap_items(&items, lines, cut(100));

  assert!(capped.kept.is_empty());
  assert_eq!(
    capped.note.as_deref(),
    Some(
      "-- one of the nodes alone is over the 100-character ceiling, so nothing was given. ask with narrower words."
    )
  );
}

// "the ceiling holds the rendering itself, a JSON one's brackets and separators included"
#[test]
fn the_ceiling_holds_the_rendering_itself() {
  let items = many("abcd", 20);
  let json = |kept: &[String]| serde_json::to_string(kept).unwrap();
  let capped = cap_items(&items, json, cut(50));

  assert!(json(capped.kept).chars().count() <= 50);
  assert!(json(&items[..capped.kept.len() + 1]).chars().count() > 50);
}

#[test]
fn counts_carry_the_thousands_mark() {
  assert_eq!(count_text(0), "0");
  assert_eq!(count_text(999), "999");
  assert_eq!(count_text(1_000), "1,000");
  assert_eq!(count_text(30_000), "30,000");
  assert_eq!(count_text(1_234_567), "1,234,567");
}
