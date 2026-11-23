//! A unit's `EnvironmentFile=` read as systemd reads one (systemd.exec(5), 「EnvironmentFile=」): one assignment a
//! line, blank lines and lines opening with `#` or `;` passed over, a value in single or double quotes taken
//! whole, and an unquoted value with its surrounding blanks taken off.
//!
//! No maintained library reads this syntax: a dotenv reader substitutes `$` and reads `#` inside a value as a
//! comment, which systemd does neither, and would hand a service a different value from the one systemd hands
//! it. So the reading is written here, for the one syntax, and stops at what systemd itself reads.

/// The assignments of an environment file's text, in their order.
pub fn assignments(text: &str) -> Vec<(String, String)> {
  let mut out = Vec::new();
  let mut lines = text.lines();

  while let Some(first) = lines.next() {
    let mut line = first.to_owned();

    // A line ending with a backslash outside quotes goes on into the next.
    while line.ends_with('\\') && !line.trim_start().starts_with(['#', ';']) {
      line.pop();

      match lines.next() {
        Some(next) => line.push_str(next),
        None => break,
      }
    }

    let line = line.trim();

    if line.is_empty() || line.starts_with(['#', ';']) {
      continue;
    }

    let Some((key, value)) = line.split_once('=') else {
      continue;
    };
    let key = key.trim();

    if key.is_empty() || !key.chars().all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
      continue;
    }

    out.push((key.to_owned(), value_of(value.trim())));
  }

  out
}

fn value_of(raw: &str) -> String {
  let quoted = |quote: char| raw.len() >= 2 && raw.starts_with(quote) && raw.ends_with(quote);

  if quoted('\'') {
    return raw[1..raw.len() - 1].to_owned();
  }

  if quoted('"') {
    let mut out = String::new();
    let mut characters = raw[1..raw.len() - 1].chars();

    while let Some(character) = characters.next() {
      match character {
        '\\' => match characters.next() {
          Some(escaped @ ('"' | '\\' | '`' | '$')) => out.push(escaped),
          Some(other) => {
            out.push('\\');
            out.push(other);
          }
          None => out.push('\\'),
        },
        other => out.push(other),
      }
    }

    return out;
  }

  raw.to_owned()
}
