//! `check` (docs/boundary.md, 「The verbs」): the check a change owes, every step to its end, or one of the
//! quick checks the declaration enables run alone (docs/checks.md). The plan, the steps and the checks are
//! the suite crate's; this is the verb's side of them.

use std::path::{Path, PathBuf};
use std::time::Instant;

use bmo_testbed_core::answer::{Cut, cap_items};
use bmo_testbed_core::declaration::{QuickCheck, Repository};
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::verbs::VERBS;
use bmo_testbed_core::turn::{Env, process_env};
use bmo_testbed_core::verb::{REFUSED_EXIT_CODE, data_root};
use bmo_testbed_suite::check::{CheckAsk, default_log, plan, run_check, run_step};
use bmo_testbed_suite::quick::language::{language_report, settle};
use bmo_testbed_suite::quick::shared_versions::shared_versions_report;
use bmo_testbed_suite::quick::{Checked, enabled_checks, quick_check};
use bmo_testbed_suite::static_check::{check_steps, run_static_check, spawn_step};
use bmo_testbed_suite::test_server::test_server_up;

use crate::own::{called_from, run_own};
use crate::recorded::command_run;
use crate::said::Said;
use crate::suite::repository;
use crate::words::{CheckArgs, CheckPart};

/// `check`: the code the call ends with.
pub fn check(args: CheckArgs) -> u8 {
  run_own(|code| fill_marks(&VERBS.own.unchecked, &[("code", &code.to_string())]));

  let repo = match repository() {
    Ok(repo) => repo,
    Err(code) => return code,
  };
  let env = process_env();

  match args.part {
    Some(part) => checked_part(part, &repo, &env, &called_from()).print(),
    None => change(args.since, args.files, args.log, args.json, &repo),
  }
}

// The static check a deployment's gate passes, alone: the steps the change since the commit reaches, each run
// to its end and said as it ends; 1 where one failed.
fn static_alone(repo: &Repository, since: Option<&str>) -> Said {
  let failed = run_static_check(
    &repo.root,
    &check_steps(repo, since),
    &mut |text| eprint!("{text}"),
    &spawn_step,
    &mut |_| {},
  );

  Said {
    out: failed.iter().cloned().collect(),
    code: u8::from(failed.is_some()),
    ..Said::default()
  }
}

// The change check over the files named, or every path changed since the commit named.
fn change(
  since: Option<String>,
  files: Vec<String>,
  log: Option<PathBuf>,
  json: bool,
  repo: &Repository,
) -> u8 {
  let said = &VERBS.check;
  let started = Instant::now();
  let ask = CheckAsk {
    since,
    files: files.iter().map(|file| file.trim_start_matches("./").to_owned()).collect(),
  };
  let plan = match plan(repo, &ask) {
    Ok(plan) => plan,
    Err(why) => {
      eprintln!("{}", fill_marks(&said.unread, &[("why", &why)]));
      return REFUSED_EXIT_CODE;
    }
  };

  // The crates' tests stand on the suite's own database server, which a run that finds it down starts.
  if !plan.touched.tested_crates.is_empty()
    && let Err(why) = test_server_up()
  {
    eprintln!("{}", fill_marks(&said.server_down, &[("why", &why)]));
  }

  let log = log.map_or_else(|| default_log(&data_root(), &repo.root), |log| called_from().join(log));

  if let Some(dir) = log.parent() {
    let _ = std::fs::create_dir_all(dir);
  }

  let checked = run_check(&repo.root, &plan, &log, &run_step);

  for line in &checked.lines {
    if json {
      eprintln!("{line}");
    } else {
      println!("{line}");
    }
  }

  if json {
    let seconds = started.elapsed().as_secs_f64();

    println!("{}", command_run(checked.code, Some(&log), checked.waited_seconds, seconds, None));
  }

  checked.code
}

// The checks' lines, and the code they end with: 1 where any found a fault.
fn printed(checks: &[Checked]) -> Said {
  Said {
    out: checks.iter().flat_map(|check| check.lines.iter().cloned()).collect(),
    code: u8::from(checks.iter().any(|check| check.faulted)),
    ..Said::default()
  }
}

/// One part of the quick checks over the repository, a path among its words read from `base`; a check the
/// declaration does not enable is refused.
pub fn checked_part(part: CheckPart, repo: &Repository, env: &Env, base: &Path) -> Said {
  let named = match &part {
    CheckPart::Quick => return printed(&enabled_checks(repo, env)),
    CheckPart::Static { since } => return static_alone(repo, since.as_deref()),
    CheckPart::Wording => QuickCheck::Wording,
    CheckPart::Layout => QuickCheck::Layout,
    CheckPart::Dependencies => QuickCheck::Dependencies,
    CheckPart::Language { .. } => QuickCheck::Language,
    CheckPart::CargoProfile => QuickCheck::CargoProfile,
    CheckPart::SharedVersions { .. } => QuickCheck::SharedVersions,
    CheckPart::Dashes { .. } => QuickCheck::Dashes,
  };

  if !repo.declaration.check.quick.contains(&named) {
    return Said {
      err: vec![fill_marks(&VERBS.check.not_enabled, &[("part", named.name())])],
      code: REFUSED_EXIT_CODE,
      ..Said::default()
    };
  }

  match part {
    CheckPart::Language { settle: true, .. } => settled(repo, env),
    CheckPart::Language { kinds, prefix, .. } if kinds || prefix.is_some() => Said {
      out: language_report(&repo.root, prefix.as_deref(), &repo.declaration.check.people_text),
      ..Said::default()
    },
    CheckPart::Dashes { files, write } => dashes(&files, base, write),
    CheckPart::SharedVersions { report: true } => reported(repo, env),
    _ => printed(&[quick_check(named, repo, env)]),
  }
}

// Every checkout of the machine's registry held to the shared versions: a line each, cut to whole lines
// under the ceiling, and the count of those that differ; 1 where any differs.
fn reported(repo: &Repository, env: &Env) -> Said {
  let words = &bmo_testbed_core::policy::quick_checks::QUICK_CHECKS.shared_versions;

  match shared_versions_report(repo, env) {
    Ok(report) => {
      let render = |kept: &[String]| kept.join("\n");
      let capped = cap_items(&report.rows, render, Cut::new(&words.report_unit, &words.report_how));
      let mut out: Vec<String> = capped.kept.to_vec();

      out.extend(capped.note);
      out.push(fill_marks(
        &words.report_total,
        &[
          ("differs", &report.differs.to_string()),
          ("all", &report.all.to_string()),
          ("file", &words.file),
        ],
      ));

      Said {
        out,
        code: u8::from(report.differs > 0),
        ..Said::default()
      }
    }
    Err(why) => Said {
      err: vec![why],
      code: 1,
      ..Said::default()
    },
  }
}

// The language ledger lowered to what each file holds now.
fn settled(repo: &Repository, env: &Env) -> Said {
  let said = &VERBS.check;
  let named = repo.declaration.check.language_ledger.clone().unwrap_or_default();
  let ledger = repo.path(&named, env);

  match settle(&repo.root, &ledger, &named.to_string_lossy()) {
    Ok(settled) => Said {
      out: vec![fill_marks(
        &said.settled,
        &[
          ("ledger", &named.to_string_lossy()),
          ("lowered", &settled.lowered.to_string()),
          ("kept", &settled.kept.to_string()),
        ],
      )],
      ..Said::default()
    },
    Err(why) => Said {
      err: vec![fill_marks(&said.unsettled, &[("why", &why)])],
      code: 1,
      ..Said::default()
    },
  }
}

/// The dashes between words of the markdown files named, read from `base`: each line that would change shown
/// before and after, and written only where `write` says so.
pub fn dashes(paths: &[String], base: &Path, write: bool) -> Said {
  let said = &VERBS.dashes;
  let full = |path: &str| base.join(path);
  let missing: Vec<&str> = paths.iter().map(String::as_str).filter(|path| !full(path).exists()).collect();

  if !missing.is_empty() {
    return Said {
      err: vec![fill_marks(&said.missing, &[("paths", &missing.join(", "))])],
      code: 1,
      ..Said::default()
    };
  }

  let mut answer = Said::default();
  let mut changed = 0;

  for path in paths {
    let before = match std::fs::read_to_string(full(path)) {
      Ok(text) => text,
      Err(why) => {
        answer
          .err
          .push(fill_marks(&said.unreadable, &[("path", path), ("why", &why.to_string())]));
        answer.code = 1;
        return answer;
      }
    };
    let after = bmo_text::dashes::without_dashes(&before, true);

    if after == before {
      continue;
    }

    let old: Vec<&str> = before.split('\n').collect();
    let lines: Vec<&str> = after.split('\n').collect();
    let moved: Vec<usize> = (0..lines.len()).filter(|at| old.get(*at) != Some(&lines[*at])).collect();

    changed += moved.len();
    answer
      .out
      .push(fill_marks(&said.file, &[("path", path), ("count", &moved.len().to_string())]));

    for at in moved {
      let line = (at + 1).to_string();

      answer.out.push(fill_marks(
        &said.before,
        &[("line", &line), ("text", old.get(at).copied().unwrap_or_default())],
      ));
      answer.out.push(fill_marks(&said.after, &[("line", &line), ("text", lines[at])]));
    }

    if write && let Err(why) = std::fs::write(full(path), &after) {
      answer
        .err
        .push(fill_marks(&said.unreadable, &[("path", path), ("why", &why.to_string())]));
      answer.code = 1;
      return answer;
    }
  }

  answer.out.push(match (changed, write) {
    (0, _) => said.none.clone(),
    (_, true) => fill_marks(&said.written, &[("count", &changed.to_string())]),
    (_, false) => fill_marks(&said.can, &[("count", &changed.to_string())]),
  });
  answer.code = u8::from(changed > 0 && !write);
  answer
}
