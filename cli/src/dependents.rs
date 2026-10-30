//! The `dependents` verb (docs/suite.md, 「The dependents of a major change」; docs/boundary.md, 「The
//! verbs」): the checkouts whose own declarations name this repository as an upstream, listed, and with `--run`
//! each consumer's suite run in turn through the suite runner.

use bmo_testbed_core::declaration::Repository;
use bmo_testbed_core::policy::dependents::DEPENDENTS;
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::turn::NOT_RUN_EXIT_CODE;
use bmo_testbed_core::verb::{REFUSED_EXIT_CODE, data_root};
use bmo_testbed_suite::dependents::{FaultKind, consumers, registry_path};
use bmo_testbed_suite::run::{SuiteAsk, run};

use crate::recorded::suite_record;
use crate::words::{BINARY, DependentsArgs};

/// What `dependents` came to: its exit code, how many consumers stood, how many failed or did not run, and the
/// one line a deployment puts beside them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DependentsEnded {
  pub code: u8,
  pub consumers: usize,
  pub failed: usize,
  pub unstarted: usize,
  pub line: String,
}

impl DependentsEnded {
  fn refused(line: String) -> Self {
    Self {
      code: REFUSED_EXIT_CODE,
      line,
      ..Self::default()
    }
  }
}

/// `dependents`: its code, and every line on standard output; a suite's own lines go to standard error as the
/// suite verb writes them.
pub fn dependents(args: &DependentsArgs) -> u8 {
  let repo = match crate::suite::repository() {
    Ok(repo) => repo,
    Err(code) => return code,
  };

  ended(&repo, args.run).code
}

/// The verb's work in `repo`, so a deployment's gate asks the same one without reading the checkout again.
pub fn ended(repo: &Repository, run_suites: bool) -> DependentsEnded {
  let said = &DEPENDENTS.lines;
  let root = data_root();
  let env = bmo_testbed_core::turn::process_env();
  let registry = registry_path(&root, &env);
  let found = match consumers(repo, &registry, &env) {
    Ok(found) => found,
    Err(fault) => {
      let line = match fault.kind {
        FaultKind::Registry => {
          fill_marks(&said.unread, &[("path", &fault.path.to_string_lossy()), ("why", &fault.why)])
        }
        FaultKind::Declaration => {
          fill_marks(&said.declaration, &[("path", &fault.path.to_string_lossy()), ("why", &fault.why)])
        }
      };

      eprintln!("{line}");

      return DependentsEnded::refused(line);
    }
  };
  let count = found.len();

  if count == 0 {
    let line =
      fill_marks(&said.none, &[("registry", &registry.to_string_lossy()), ("repository", &repo.name())]);

    println!("{line}");

    return DependentsEnded {
      line,
      ..DependentsEnded::default()
    };
  }

  println!(
    "{}",
    fill_marks(&said.heading, &[("count", &count.to_string()), ("repository", &repo.name())])
  );

  for consumer in &found {
    println!(
      "{}",
      fill_marks(&said.entry, &[("name", &consumer.name()), ("root", &consumer.root.to_string_lossy())])
    );
  }

  if !run_suites {
    return DependentsEnded {
      consumers: count,
      ..DependentsEnded::default()
    };
  }

  let mut failed = 0;
  let mut unstarted = 0;

  for consumer in &found {
    let name = consumer.name();

    eprintln!(
      "{}",
      fill_marks(&said.running, &[("name", &name), ("root", &consumer.root.to_string_lossy())])
    );

    let ask = SuiteAsk {
      whole_guard: consumer.declaration.test.guard_seconds,
      file_guard: consumer.declaration.test.file_guard_seconds,
      record: Some(suite_record(&root, consumer, format!("{BINARY} dependents --run"))),
      ..SuiteAsk::default()
    };
    let ended = run(consumer, &ask);

    if ended.code == 0 {
      println!("{}", fill_marks(&said.pass, &[("name", &name)]));
    } else if ended.code == NOT_RUN_EXIT_CODE {
      unstarted += 1;
      println!("{}", fill_marks(&said.unstarted, &[("name", &name), ("code", &ended.code.to_string())]));
    } else {
      failed += 1;
      let log = ended.log.as_ref().map_or_else(String::new, |log| log.to_string_lossy().into_owned());

      println!(
        "{}",
        fill_marks(&said.fail, &[("name", &name), ("code", &ended.code.to_string()), ("log", &log)])
      );
    }
  }

  let (code, line) = if failed > 0 {
    (
      1,
      fill_marks(&said.failed_some, &[("failed", &failed.to_string()), ("count", &count.to_string())]),
    )
  } else if unstarted > 0 {
    (
      NOT_RUN_EXIT_CODE,
      fill_marks(
        &said.unstarted_some,
        &[("unstarted", &unstarted.to_string()), ("count", &count.to_string())],
      ),
    )
  } else {
    (0, fill_marks(&said.passed_all, &[("count", &count.to_string())]))
  };

  println!("{line}");

  DependentsEnded {
    code,
    consumers: count,
    failed,
    unstarted,
    line,
  }
}
