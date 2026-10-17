//! The ceiling every verb answers under (docs/boundary.md, 「The binary」): an answer that would be longer is
//! cut to whole items, and one line says how many of the things asked for are in it, how many are not, and
//! how to ask for the rest.

use crate::policy::answer::ANSWER;
use crate::policy::fill_marks;

/// How a verb names what it cut, in its own terms: the things it counts, how to ask for the rest, and the
/// ceiling where it is not the standing one.
#[derive(Clone, Copy, Debug)]
pub struct Cut<'a> {
  pub unit: &'a str,
  pub how: &'a str,
  pub limit: usize,
}

impl<'a> Cut<'a> {
  pub fn new(unit: &'a str, how: &'a str) -> Self {
    Self {
      unit,
      how,
      limit: ANSWER.ceiling_chars,
    }
  }
}

/// What fitted under the ceiling, and what had to be left out.
#[derive(Debug, PartialEq)]
pub struct Capped<'a, T> {
  /// The run of items from the front that fits, in the order they were given.
  pub kept: &'a [T],
  /// How many did not; nought where everything fitted.
  pub left: usize,
  /// The one line saying so, or none where nothing was left out.
  pub note: Option<String>,
}

// Characters are code points, the units a reader counts in.
fn chars(text: &str) -> usize {
  text.chars().count()
}

/// A count with the thousands mark.
pub fn count_text(value: usize) -> String {
  let digits = value.to_string();
  let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);

  for (at, digit) in digits.chars().enumerate() {
    if at > 0 && (digits.len() - at).is_multiple_of(3) {
      grouped.push(',');
    }

    grouped.push(digit);
  }

  grouped
}

/// The longest run of `items` from the front whose rendering fits under the ceiling, with what was left out
/// named in the caller's terms. `render` is handed a run and returns exactly what the verb would print for
/// it, so the ceiling holds the answer itself, separators and all. Where even one item is over the ceiling
/// nothing is kept: half an item is not an item.
pub fn cap_items<'a, T>(items: &'a [T], render: impl Fn(&[T]) -> String, cut: Cut<'_>) -> Capped<'a, T> {
  if chars(&render(items)) <= cut.limit {
    return Capped {
      kept: items,
      left: 0,
      note: None,
    };
  }

  // The largest run that fits, found by halving: a longer run is a longer answer for every rendering.
  let mut fits = 0;
  let mut over = items.len();

  while over - fits > 1 {
    let middle = (fits + over) / 2;

    if chars(&render(&items[..middle])) <= cut.limit {
      fits = middle;
    } else {
      over = middle;
    }
  }

  let left = items.len() - fits;
  let limit = count_text(cut.limit);
  let note = if fits == 0 {
    fill_marks(&ANSWER.alone, &[("unit", cut.unit), ("limit", &limit), ("how", cut.how)])
  } else {
    fill_marks(
      &ANSWER.cut,
      &[
        ("limit", &limit),
        ("kept", &count_text(fits)),
        ("all", &count_text(items.len())),
        ("unit", cut.unit),
        ("left", &count_text(left)),
        ("how", cut.how),
      ],
    )
  };

  Capped {
    kept: &items[..fits],
    left,
    note: Some(note),
  }
}
