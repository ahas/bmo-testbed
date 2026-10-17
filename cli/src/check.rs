//! `check` (docs/boundary.md, 「The verbs」): the check a change owes, every step to its end, or one of the
//! quick checks the declaration enables run alone (docs/checks.md). The plan, the steps and the checks are
//! the suite crate's; this is the verb's side of them.

use std::path::{Path, PathBuf};

use bmo_testbed_core::declaration::{QuickCheck, Repository};
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::verbs::VERBS;
use bmo_testbed_core::turn::{Env, process_env};
use bmo_testbed_core::verb::{REFUSED_EXIT_CODE, data_root};
use bmo_testbed_suite::check::{CheckAsk, default_log, plan, run_check, run_step};
use bmo_testbed_suite::quick::korean::{korean_report, settle};
use bmo_testbed_suite::quick::rewrite::without_dashes;
use bmo_testbed_suite::quick::{Checked, enabled_checks, quick_check};
use bmo_testbed_suite::test_server::test_server_up;

use crate::own::{called_from, run_own};
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
    None => change(args.since, args.files, args.log, &repo),
  }
}

// The change check over the files named, or every path changed since the commit named.
fn change(since: Option<String>, files: Vec<String>, log: Option<PathBuf>, repo: &Repository) -> u8 {
  let said = &VERBS.check;
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
    println!("{line}");
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
    CheckPart::Wording => QuickCheck::Wording,
    CheckPart::Layout => QuickCheck::Layout,
    CheckPart::Dependencies => QuickCheck::Dependencies,
    CheckPart::Korean { .. } => QuickCheck::Korean,
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
    CheckPart::Korean { settle: true, .. } => settled(repo, env),
    CheckPart::Korean { kinds, prefix, .. } if kinds || prefix.is_some() => Said {
      out: korean_report(&repo.root, prefix.as_deref(), &repo.declaration.check.people_text),
      ..Said::default()
    },
    CheckPart::Dashes { files, write } => dashes(&files, base, write),
    _ => printed(&[quick_check(named, repo, env)]),
  }
}

// The Korean ledger lowered to what each file holds now.
fn settled(repo: &Repository, env: &Env) -> Said {
  let said = &VERBS.check;
  let named = repo.declaration.check.korean_ledger.clone().unwrap_or_default();
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
    let after = without_dashes(&before);

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
