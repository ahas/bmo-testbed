//! The suite's verbs (docs/boundary.md, 「The verbs」): `suite`, a run of the repository's units placed like
//! any heavy run and judged once more on asking; `count`, the tests a range removed; `log`, what a run's log
//! came to; and `template`, the suite's template database.

use std::path::Path;

use bmo_testbed_core::answer::{Cut, cap_items};
use bmo_testbed_core::declaration::{Repository, root_of};
use bmo_testbed_core::events::Hook;
use bmo_testbed_core::placement::{PlaceAsk, runs_here};
use bmo_testbed_core::policy::count::COUNT;
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::suite::SUITE;
use bmo_testbed_core::policy::verbs::VERBS;
use bmo_testbed_core::turn::{Lane, process_env};
use bmo_testbed_core::verb::{REFUSED_EXIT_CODE, data_root};
use bmo_testbed_suite::count::{GitRange, count_range, tree_count};
use bmo_testbed_suite::log::{
  failed_files_of, failed_tests_of, failures_of, tally_of, unstarted_of, verdict_of,
};
use bmo_testbed_suite::once_more::run_judged;
use bmo_testbed_suite::run::{SuiteAsk, SuiteWords, ask_of, run, verdict_line};
use bmo_testbed_suite::template::{TemplateVerb, template as template_on};
use serde_json::json;

use crate::own::run_own;
use crate::placement_of;
use crate::recorded::{command_run, suite_record};
use crate::said::Said;
use crate::words::{BINARY, CountArgs, LogArgs, SuiteArgs, TemplateArgs, TemplateWord};

/// The repository the verb runs in, or the code it ends with where its declaration does not read.
pub fn repository() -> Result<Repository, u8> {
  Repository::read(&std::env::current_dir().unwrap_or_default()).map_err(|refusal| {
    eprintln!("{refusal}");
    REFUSED_EXIT_CODE
  })
}

/// `suite`: the code the run ends with.
pub fn suite(args: SuiteArgs) -> u8 {
  run_own(|code| {
    format!(
      "\n{}",
      verdict_line(false, 0.0, &fill_marks(&SUITE.ending.unbuilt, &[("code", &code.to_string())]))
    )
  });

  let repo = match repository() {
    Ok(repo) => repo,
    Err(code) => return code,
  };
  let given = SuiteWords {
    since: args.since.clone(),
    words: args.words.clone(),
  };
  let command: Vec<String> = std::iter::once(BINARY.to_owned()).chain(std::env::args().skip(1)).collect();
  let ask = SuiteAsk {
    record: Some(suite_record(&data_root(), &repo, command.join(" "))),
    ..ask_of(&given, &repo.declaration.test)
  };
  let env = process_env();

  // A placed suite runs on the other machine as this same verb of the testbed there, on the same words.
  if !runs_here(&env) {
    let placed = placement_of(&env, &repo.root).place(&PlaceAsk {
      command: &command,
      lane: Lane::Wide,
      cwd: std::env::current_dir().unwrap_or_default(),
      env: &env,
    });

    if let Some(code) = placed {
      if args.json {
        println!("{}", command_run(code, None, 0.0, 0.0, None));
      }
      return code;
    }
  }

  let ended = if args.again {
    run_judged(&repo, &ask, &data_root(), &Hook::of(Some(&repo), &env)).0
  } else {
    run(&repo, &ask)
  };

  if args.json {
    println!(
      "{}",
      command_run(ended.code, ended.log.as_deref(), ended.waited_seconds, ended.seconds, ended.test_run)
    );
  }

  ended.code
}

/// `count`: the tests the checkout holds, and with a range the tests it removed, each accounted for or not.
pub fn count(args: &CountArgs, dir: &Path) -> Said {
  let said = &COUNT;
  let root = root_of(dir).unwrap_or_else(|| dir.to_path_buf());
  let failed = |why: String| Said {
    err: vec![fill_marks(&said.failed, &[("why", &why)])],
    code: 1,
    ..Said::default()
  };
  let (typescript, rust) = match tree_count(&root) {
    Ok(counted) => counted,
    Err(why) => return failed(why),
  };
  let mut answer = Said {
    out: vec![fill_marks(
      &said.tree,
      &[("typescript", &typescript.to_string()), ("rust", &rust.to_string())],
    )],
    ..Said::default()
  };
  let Some(since) = args.since.as_deref() else { return answer };
  let range = match count_range(&GitRange { root: &root, since }) {
    Ok(range) => range,
    Err(why) => return failed(why),
  };

  answer.out.push(fill_marks(
    &said.range,
    &[
      ("since", since),
      ("before", &range.before.to_string()),
      ("after", &range.after.to_string()),
      ("removed", &range.removed.len().to_string()),
    ],
  ));
  answer.out.extend(
    range
      .removed
      .iter()
      .map(|one| fill_marks(&said.removed, &[("file", &one.file), ("title", &one.title)])),
  );
  answer.err.extend(
    range
      .unaccounted
      .iter()
      .map(|one| fill_marks(&said.open, &[("file", &one.file), ("title", &one.title)])),
  );
  answer.code = u8::from(!range.unaccounted.is_empty());
  answer
}

// A list of names under a heading, cut to whole names where the list, with `around` characters of the answer
// standing before and after it, would take the answer past the ceiling.
fn named_list(heading: &str, names: &[String], around: usize) -> Vec<String> {
  let said = &VERBS.log;
  let render = |kept: &[String]| {
    std::iter::once(heading.to_owned())
      .chain(kept.iter().map(|one| fill_marks(&said.item, &[("name", one)])))
      .collect::<Vec<_>>()
      .join("\n")
  };
  let capped = cap_items(
    names,
    |kept| format!("{}{}", " ".repeat(around), render(kept)),
    Cut::new(&said.unit, &said.how),
  );
  let mut lines = vec![render(capped.kept)];

  lines.extend(capped.note);
  lines
}

/// `log`: what the log at `path`, read from `base` where it is not absolute, came to.
pub fn log(args: &LogArgs, base: &Path) -> Said {
  let said = &VERBS.log;
  let shown = args.log.to_string_lossy();
  let Ok(log) = std::fs::read_to_string(base.join(&args.log)) else {
    return Said {
      err: vec![fill_marks(&said.missing, &[("path", &shown)])],
      code: 1,
      ..Said::default()
    };
  };
  let tally = tally_of(&log);
  let unstarted = unstarted_of(&log);
  let verdict = verdict_of(&log);
  let summarised = tally.pass > 0 || tally.fail > 0 || verdict.is_some();
  let failures = failures_of(&log);
  let code = u8::from(tally.fail > 0 || !failures.is_empty() || !unstarted.is_empty() || !summarised);

  if args.json {
    let units = failed_files_of(&log);
    let placed = failed_tests_of(&log);
    // Each failing test with the unit the log gives it, then each the log names under no unit, null there;
    // a name that is a unit's own is the unit, not a test.
    let tests: Vec<(String, Option<String>)> = placed
      .iter()
      .map(|one| (one.test.clone(), Some(one.file.clone())))
      .chain(
        failures
          .iter()
          .filter(|name| !units.contains(name) && !placed.iter().any(|one| &one.test == *name))
          .map(|name| (name.clone(), None)),
      )
      .collect();
    let render = |kept: &[(String, Option<String>)]| {
      let failures: Vec<_> =
        kept.iter().map(|(test, unit)| json!({ "test": test, "unit": unit })).collect();

      json!({
        "passed": tally.pass,
        "failed": tally.fail,
        "skipped": tally.skip,
        "verdict": verdict,
        "failed_units": units,
        "unstarted_units": unstarted,
        "failures": failures,
      })
      .to_string()
    };
    let capped = cap_items(&tests, render, Cut::new(&said.unit, &said.how));

    return Said {
      out: vec![render(capped.kept)],
      err: capped.note.into_iter().collect(),
      code,
    };
  }

  let mut out: Vec<String> = verdict.iter().cloned().collect();
  let counted: Vec<String> = [
    ("pass", tally.pass, true),
    ("fail", tally.fail, true),
    ("skip", tally.skip, false),
    ("todo", tally.todo, false),
    ("files", tally.files, false),
  ]
  .into_iter()
  .filter(|(_, count, always)| *always || *count > 0)
  .map(|(name, count, _)| fill_marks(&said.count, &[("name", name), ("count", &count.to_string())]))
  .collect();

  out.push(counted.join(&said.joiner));

  // The failures take what the ceiling leaves past the head and the share kept back for the units never
  // started, which take whatever is left after the failures.
  let used = |lines: &[String]| lines.iter().map(|line| line.chars().count() + 1).sum::<usize>();
  let kept_back = bmo_testbed_core::policy::answer::ANSWER.ceiling_chars / said.unstarted_divisor;

  if !failures.is_empty() {
    let heading = format!("\n{}", fill_marks(&said.failures, &[("count", &failures.len().to_string())]));

    out.extend(named_list(&heading, &failures, used(&out) + kept_back));
  }

  // Nothing of a unit the run never started was tested, so it is no failure, and the run still has no result
  // for it.
  if !unstarted.is_empty() {
    let heading = format!("\n{}", fill_marks(&said.unstarted, &[("count", &unstarted.len().to_string())]));

    out.extend(named_list(&heading, &unstarted, used(&out)));
  }

  Said {
    out,
    err: if summarised {
      Vec::new()
    } else {
      vec![format!("\n{}", fill_marks(&said.no_summary, &[("path", &shown)]))]
    },
    code,
  }
}

/// `template`: the code the verb ends with, 2 where the declaration names no migrate command to make one.
pub fn template(args: &TemplateArgs) -> u8 {
  run_own(|code| fill_marks(&VERBS.own.untouched, &[("code", &code.to_string())]));

  let repo = match repository() {
    Ok(repo) => repo,
    Err(code) => return code,
  };
  let verb = match args.verb {
    TemplateWord::Make => TemplateVerb::Make,
    TemplateWord::Drop => TemplateVerb::Drop,
    TemplateWord::Sweep => TemplateVerb::Sweep,
  };
  let failed = |why: &str| eprintln!("{}", fill_marks(&SUITE.template.failed, &[("why", why)]));
  let migrates = repo
    .declaration
    .test
    .database
    .as_ref()
    .is_some_and(|database| database.migrate.is_some());

  if verb == TemplateVerb::Make && !migrates {
    failed(&SUITE.template.no_migrate);
    return REFUSED_EXIT_CODE;
  }

  match template_on(verb, &args.name, &repo) {
    Ok(()) => 0,
    Err(why) => {
      failed(&why);
      1
    }
  }
}
