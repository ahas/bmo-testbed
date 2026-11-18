//! The `close-line` verb (docs/boundary.md, 「`close-line`」): the close lines of the named checkouts'
//! repositories, taken in the one order every holder of several keeps and held by this process until its
//! standard input ends, for a close run outside the testbed.

use std::io::Write;
use std::path::{Path, PathBuf};

use bmo_testbed_core::close_line::{CloseLineAsk, CloseTurn, close_line_path, take_close_line};
use bmo_testbed_core::policy::close_line::CLOSE_LINE;
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::turn::{Env, NOT_RUN_EXIT_CODE, process_env};
use bmo_testbed_core::verb::{REFUSED_EXIT_CODE, data_root};

use crate::words::CloseLineArgs;

/// A line to take: where its lock stands, and the checkout it is taken for.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct LineToTake {
  pub lock: PathBuf,
  pub checkout: String,
}

/// The lines the checkouts named from `cwd` stand in under `data_root`, in the order they are taken: by their
/// lock files' names, each repository's line once. The path that names no checkout of a git repository where one
/// does not.
pub fn lines_to_take(
  data_root: &Path,
  cwd: &Path,
  checkouts: &[String],
) -> Result<Vec<LineToTake>, String> {
  let mut lines = Vec::new();

  for given in checkouts {
    let checkout = cwd.join(given);
    let lock = close_line_path(data_root, &checkout).ok_or_else(|| given.clone())?;

    lines.push(LineToTake {
      lock,
      checkout: checkout.to_string_lossy().into_owned(),
    });
  }

  lines.sort();
  lines.dedup_by(|later, earlier| later.lock == earlier.lock);
  Ok(lines)
}

/// Every line taken one after another, waited for however long each takes; none where the harness went away
/// before every line came, what was already held let go.
pub fn take_lines(lines: &[LineToTake], env: &Env, say: &dyn Fn(&str)) -> Option<Vec<CloseTurn>> {
  lines
    .iter()
    .map(|line| {
      take_close_line(&CloseLineAsk {
        path: line.lock.clone(),
        env: env.clone(),
        checkout: line.checkout.clone(),
        say,
        notice_seconds: None,
        poll: None,
        watch: true,
      })
    })
    .collect()
}

/// The verb: the lines taken, one line said once all are held, then held until standard input ends.
pub fn close_line(args: &CloseLineArgs) -> u8 {
  let words = &CLOSE_LINE.hold;
  let cwd = std::env::current_dir().unwrap_or_default();
  let lines = match lines_to_take(&data_root(), &cwd, &args.checkouts) {
    Ok(lines) => lines,
    Err(path) => {
      eprintln!("{}", fill_marks(&words.not_a_checkout, &[("path", &path)]));
      return REFUSED_EXIT_CODE;
    }
  };
  let say = |line: &str| eprintln!("{line}");
  let Some(held) = take_lines(&lines, &process_env(), &say) else {
    eprintln!("{}", words.gone);
    return NOT_RUN_EXIT_CODE;
  };
  let checkouts: Vec<&str> = lines.iter().map(|line| line.checkout.as_str()).collect();
  let mut out = std::io::stdout();

  let _ = writeln!(
    out,
    "{}",
    fill_marks(
      &words.held,
      &[("count", &held.len().to_string()), ("checkouts", &checkouts.join(&words.joiner))]
    )
  );
  let _ = out.flush();
  // Held until the consumer closes its end of standard input, or it ends.
  let _ = std::io::copy(&mut std::io::stdin().lock(), &mut std::io::sink());
  drop(held);
  0
}
