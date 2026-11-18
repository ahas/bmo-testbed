//! `check` (docs/boundary.md, 「The verbs」): the check a change owes, every step to its end, or one of the
//! quick checks the declaration enables run alone (docs/checks.md). The plan, the steps and the checks are
//! the suite crate's; this is the verb's side of them.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use bmo_testbed_client::TestRunKind;
use bmo_testbed_core::answer::{Cut, cap_items};
use bmo_testbed_core::declaration::{QuickCheck, Repository, root_of};
use bmo_testbed_core::placement::{PlaceAsk, SetAsk, runs_here};
use bmo_testbed_core::policy::suite::SUITE;
use bmo_testbed_core::policy::verbs::VERBS;
use bmo_testbed_core::policy::{fill_marks, fill_shown};
use bmo_testbed_core::turn::{Env, Lane, NOT_RUN_EXIT_CODE, TurnAsk, process_env, take_turn};
use bmo_testbed_core::verb::{REFUSED_EXIT_CODE, data_root};
use bmo_testbed_suite::check::{
  CheckAsk, SetMember, default_log, default_set_log, default_since, plan, run_check, run_set, run_standing,
  run_step,
};
use bmo_testbed_suite::quick::language::{language_report, settle};
use bmo_testbed_suite::quick::shared_versions::shared_versions_report;
use bmo_testbed_suite::quick::{Checked, enabled_checks, quick_check};
use bmo_testbed_suite::static_check::{check_steps, run_static_check, spawn_step};
use bmo_testbed_suite::test_server::test_server_up;
use bmo_testbed_suite::workspace::git_lines;

use crate::own::{called_from, run_own};
use crate::recorded::{Recorded, command_run};
use crate::said::Said;
use crate::suite::repository;
use crate::words::{BINARY, CheckArgs, CheckPart};
use crate::{placement_of, set_placement_of};

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
    None if !args.checkouts.is_empty() => set_change(&args.checkouts, args.log, args.json, &env),
    None => placed_change(args.since.as_deref(), &args.files, args.json, &repo, &env)
      .unwrap_or_else(|| change(args.since, args.files, args.log, args.json, &repo)),
  }
}

// The change check placed whole where the machine's placement sends it elsewhere (docs/testbed.md, 「A run on
// another machine」): the code it ended with there, or none where it runs here.
fn placed_change(
  since: Option<&str>,
  files: &[String],
  json: bool,
  repo: &Repository,
  env: &Env,
) -> Option<u8> {
  if runs_here(env) {
    return None;
  }

  let mut command = vec![BINARY.to_owned(), "check".to_owned()];

  // The commit named here, as the commit itself: the bench there holds none of this checkout's references.
  if let Some(since) = since {
    command.extend(["--since".to_owned(), resolved_commit(&repo.root, since)]);
  }

  if json {
    command.push("--json".to_owned());
  }

  command.extend(files.iter().cloned());

  placement_of(env, &repo.root).place(&PlaceAsk {
    command: &command,
    adapter: None,
    lane: Lane::Wide,
    cwd: called_from(),
    env,
  })
}

// The commit a name resolves to in the checkout; the name as given where it resolves to none, which the check it
// is handed to then refuses.
fn resolved_commit(root: &Path, named: &str) -> String {
  commit_named(root, named).unwrap_or_else(|| named.to_owned())
}

// The commit a name resolves to in the checkout; none where it resolves to none.
fn commit_named(root: &Path, named: &str) -> Option<String> {
  git_lines(root, &["rev-parse", "--verify", "--quiet", &format!("{named}^{{commit}}")])
    .ok()
    .and_then(|lines| lines.into_iter().next())
}

/// A `--checkout` word read from `base` (docs/checks.md, 「The change check over several checkouts」): a word
/// naming a directory that stands is a path with no since; any other is split at its first `@`, the path
/// before it and the since after it.
pub fn checkout_word<'a>(word: &'a str, base: &Path) -> (&'a str, Option<&'a str>) {
  if base.join(word).is_dir() {
    return (word, None);
  }

  match word.split_once('@') {
    Some((path, since)) => (path, Some(since)),
    None => (word, None),
  }
}

// Every `--checkout` read before anything runs: its repository at its top level and the commit its change is
// read since, the since named or its merge base with its declared main; the line saying which word does not
// read where one does not.
fn read_checkouts(words: &[String], base: &Path) -> Result<Vec<(Repository, String)>, String> {
  let said = &VERBS.check;
  let mut read: Vec<(Repository, String)> = Vec::with_capacity(words.len());

  for word in words {
    let (path, since) = checkout_word(word, base);
    let Some(root) = root_of(&base.join(path)) else {
      return Err(fill_marks(&said.no_checkout, &[("word", word)]));
    };
    let repo = Repository::read(&root).map_err(|refusal| refusal.to_string())?;

    if read.iter().any(|(known, _)| known.root == repo.root) {
      return Err(fill_marks(&said.twice, &[("word", word)]));
    }

    let named = since.map_or_else(|| default_since(&repo), str::to_owned);
    let Some(commit) = commit_named(&repo.root, &named) else {
      return Err(fill_marks(&said.no_commit, &[("word", word), ("since", &named)]));
    };

    read.push((repo, commit));
  }

  Ok(read)
}

// The words naming each checkout with its since, as a `--checkout` of the change check over several checkouts.
fn checkout_words<'a>(paths: impl IntoIterator<Item = (String, &'a String)>, json: bool) -> Vec<String> {
  let mut words = vec!["check".to_owned()];

  for (path, since) in paths {
    words.extend(["--checkout".to_owned(), format!("{path}@{since}")]);
  }

  if json {
    words.push("--json".to_owned());
  }

  words
}

// The change check over several checkouts (docs/checks.md, 「The change check over several checkouts」): every
// checkout read first, the set placed as one run where the machine's placement sends it elsewhere or refuses
// it, and otherwise run here in one wide turn.
fn set_change(words: &[String], log: Option<PathBuf>, json: bool, env: &Env) -> u8 {
  let base = called_from();
  let read = match read_checkouts(words, &base) {
    Ok(read) => read,
    Err(line) => {
      eprintln!("{line}");
      return REFUSED_EXIT_CODE;
    }
  };

  if !runs_here(env) {
    let roots: Vec<PathBuf> = read.iter().map(|(repo, _)| repo.root.clone()).collect();
    let sinces: Vec<&String> = read.iter().map(|(_, since)| since).collect();
    let command: Vec<String> = std::iter::once(BINARY.to_owned())
      .chain(checkout_words(
        read.iter().map(|(repo, since)| (repo.root.display().to_string(), since)),
        json,
      ))
      .collect();
    let far = |paths: &[String]| checkout_words(paths.iter().cloned().zip(sinces.iter().copied()), json);
    let ask = SetAsk {
      run: PlaceAsk {
        command: &command,
        adapter: None,
        lane: Lane::Wide,
        cwd: base.clone(),
        env,
      },
      checkouts: &roots,
      words: &far,
    };

    if let Some(code) = set_placement_of(env, &roots).place_set(&ask) {
      return code;
    }
  }

  set_here(read, log, json, env)
}

// The set run here: every checkout planned by its own declaration, one wide turn taken for the whole set and
// handed to every step, the checkouts run in turn, each kept as one test run of kind `check`.
fn set_here(read: Vec<(Repository, String)>, log: Option<PathBuf>, json: bool, env: &Env) -> u8 {
  let said = &VERBS.check;
  let started = Instant::now();
  let data = data_root();
  let mut members = Vec::with_capacity(read.len());

  for (repo, since) in read {
    let plan = match plan(
      &repo,
      &CheckAsk {
        since: Some(since.clone()),
        files: vec![],
      },
    ) {
      Ok(plan) => plan,
      Err(why) => {
        let why = fill_shown(&SUITE.check.set_line, &[("repository", &repo.name()), ("line", &why)]);

        eprintln!("{}", fill_marks(&said.unread, &[("why", &why)]));
        return REFUSED_EXIT_CODE;
      }
    };

    members.push(SetMember {
      command: checkout_words([(repo.root.display().to_string(), &since)], false).join(" "),
      log: default_log(&data, &repo.root),
      repo,
      plan,
    });
  }

  if members.iter().any(|member| !member.plan.touched.tested_crates.is_empty())
    && let Err(why) = test_server_up()
  {
    eprintln!("{}", fill_marks(&said.server_down, &[("why", &why)]));
  }

  let roots: Vec<PathBuf> = members.iter().map(|member| member.repo.root.clone()).collect();
  let log = log.map_or_else(|| default_set_log(&data, &roots), |log| called_from().join(log));

  if let Some(dir) = log.parent() {
    let _ = std::fs::create_dir_all(dir);
  }

  let command = members
    .iter()
    .map(|member| member.command.as_str())
    .collect::<Vec<_>>()
    .join(&SUITE.check.set_steps_joiner);
  let turn = take_turn(&TurnAsk {
    command: format!("{BINARY} {command}"),
    lane: Lane::Wide,
    say: &|line| eprintln!("{line}"),
    path: None,
    env: env.clone(),
    checkout: roots.first().map(|root| root.to_string_lossy().into_owned()).unwrap_or_default(),
    poll: None,
    notice_seconds: None,
  });

  if !turn.had() {
    eprintln!("{}", fill_shown(&SUITE.check.no_turn, &[("words", &turn.words())]));

    if json {
      let seconds = started.elapsed().as_secs_f64();

      println!("{}", command_run(NOT_RUN_EXIT_CODE, Some(&log), turn.waited_seconds(), seconds, None));
    }

    return NOT_RUN_EXIT_CODE;
  }

  // Every step runs inside the set's one turn.
  let held: Vec<(String, String)> =
    turn.held_variable().map(|(name, value)| (name.to_owned(), value)).into_iter().collect();
  let every = Duration::from_secs(SUITE.numbers.check_standing_seconds);
  let checked = run_set(
    &members,
    &log,
    env,
    &|root, step| {
      let mut step = step.clone();

      step.step.env.extend(held.iter().cloned());
      run_standing(&step, every, &|line| eprintln!("{line}"), || run_step(root, &step))
    },
    &|repo| {
      Box::new(Recorded {
        data_root: data.clone(),
        kind: TestRunKind::Check,
        repository_name: Some(repo.name()),
      })
    },
    &mut |line| {
      if json {
        eprintln!("{line}");
      } else {
        println!("{line}");
      }
    },
  );

  if json {
    let seconds = started.elapsed().as_secs_f64();
    let waited = checked.waited_seconds + turn.waited_seconds();

    println!("{}", command_run(checked.code, Some(&log), waited, seconds, None));
  }

  drop(turn);
  checked.code
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

  let every = Duration::from_secs(SUITE.numbers.check_standing_seconds);
  let checked = run_check(&repo.root, &plan, &log, &|root, step| {
    run_standing(step, every, &|line| eprintln!("{line}"), || run_step(root, step))
  });

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
    CheckPart::PersonalData => QuickCheck::PersonalData,
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
