//! One command run in the machine's turn and read into the testbed's one result (docs/testbed.md, 「The
//! result」): whichever language runs it, the same line — passed or not, the counts, the time it ran and the
//! time it waited, the log — and what failed under it.

use std::fs::File;
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

use crate::languages::{Language, PLAIN, Tally, failures_of_log, tally_of_log};
use crate::policy::fill_marks;
use crate::policy::lanes::LANES;
use crate::queue::now_ms;
use crate::turn::{NOT_RUN_EXIT_CODE, TurnAsk, lane_width, machine_threads, record_turn_times, take_turn};
use crate::witness::{RunBegun, RunEnded, RunVerdict, RunWitness};

/// How one run ended.
#[derive(Clone, Debug, PartialEq)]
pub struct TestbedResult {
  pub language: &'static str,
  /// Not had: the turn did not come, nothing ran.
  pub ran: bool,
  pub ok: bool,
  pub code: Option<i32>,
  pub tally: Option<Tally>,
  pub seconds: f64,
  pub waited_seconds: f64,
  pub log: Option<PathBuf>,
  pub failures: Vec<String>,
  /// The turn, as its caller says it once settled.
  pub turn: String,
  /// The id the witness kept the run under, where it kept it.
  pub test_run: Option<i64>,
}

impl TestbedResult {
  /// The one line a testbed run ends on.
  pub fn line(&self) -> String {
    let said = &LANES.run;

    if !self.ran {
      return fill_marks(&said.not_run, &[("turn", &self.turn)]);
    }

    let counts = self.tally.map_or_else(
      || {
        let code = self.code.map_or_else(|| said.by_signal.clone(), |code| code.to_string());

        fill_marks(&said.exit, &[("code", &code)])
      },
      |tally| {
        fill_marks(
          &said.counts,
          &[
            ("pass", &tally.pass.to_string()),
            ("fail", &tally.fail.to_string()),
            ("skip", &tally.skip.to_string()),
          ],
        )
      },
    );
    let log = self.log.as_ref().map_or_else(String::new, |log| log.to_string_lossy().into_owned());

    fill_marks(
      &said.line,
      &[
        ("verdict", if self.ok { &said.ok } else { &said.failed }),
        ("seconds", &self.seconds.round().to_string()),
        ("waited", &self.waited_seconds.round().to_string()),
        ("language", self.language),
        ("counts", &counts),
        ("log", &log),
      ],
    )
  }

  /// The exit code a run ends the caller on.
  pub fn exit_code(&self) -> u8 {
    if !self.ran {
      return NOT_RUN_EXIT_CODE;
    }

    if self.ok {
      return 0;
    }

    self
      .code
      .filter(|code| *code != 0)
      .and_then(|code| u8::try_from(code).ok())
      .unwrap_or(1)
  }
}

/// A run asked for.
pub struct RunAsk<'a> {
  pub command: Vec<String>,
  /// The language that runs it, and every language, which read its log together.
  pub language: &'static dyn Language,
  pub languages: &'a [&'static dyn Language],
  pub log_dir: PathBuf,
  pub cwd: PathBuf,
  /// The turn's own ask, its lane the one the command runs in, at that lane's width.
  pub turn: TurnAsk<'a>,
  /// Whoever keeps the record of the run, told as it begins and as it ends.
  pub witness: &'a dyn RunWitness,
}

// The logs older than the days the policy keeps them, taken away as the next run starts.
fn prune_logs(dir: &Path, now: SystemTime) {
  let keep = Duration::from_secs(LANES.numbers.log_keep_days * 24 * 60 * 60);

  for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
    let old = entry
      .metadata()
      .and_then(|meta| meta.modified())
      .ok()
      .and_then(|modified| now.duration_since(modified).ok())
      .is_some_and(|age| age > keep);

    // Taken by another run's prune between the listing and the look, where it fails.
    if old {
      let _ = std::fs::remove_file(entry.path());
    }
  }
}

/// A run's log, named by when it started, what runs it and this process.
pub fn log_name(started_ms: i64, language: &str) -> String {
  let at = chrono::DateTime::from_timestamp_millis(started_ms)
    .map_or_else(|| started_ms.to_string(), |at| at.format("%Y-%m-%dT%H-%M-%S-%3fZ").to_string());

  format!("{at}-{language}-{}.log", std::process::id())
}

/// The command run in its turn, wide, until it ends, its output into a log of its own; or the words of
/// why it could not run at all.
pub fn run_in_testbed(ask: &RunAsk<'_>) -> Result<TestbedResult, String> {
  let turn = take_turn(&ask.turn);
  let not_run = || TestbedResult {
    language: ask.language.name(),
    ran: false,
    ok: false,
    code: None,
    tally: None,
    seconds: 0.0,
    waited_seconds: turn.waited_seconds(),
    log: None,
    failures: Vec::new(),
    turn: turn.words(),
    test_run: None,
  };

  if !turn.had() {
    return Ok(not_run());
  }

  let started = Instant::now();
  let said = &LANES.run;
  let no_log = |why: std::io::Error| {
    fill_marks(&said.no_log, &[("dir", &ask.log_dir.to_string_lossy()), ("why", &why.to_string())])
  };

  std::fs::create_dir_all(&ask.log_dir).map_err(no_log)?;
  prune_logs(&ask.log_dir, SystemTime::now());

  let started_at = now_ms();
  let log = ask.log_dir.join(log_name(started_at, ask.language.name()));
  let out = File::create(&log).map_err(no_log)?;
  let test_run = ask.witness.began(&RunBegun {
    command: &ask.turn.command,
    checkout: &ask.cwd,
    log: &log,
    started_at,
    env: &ask.turn.env,
  });
  let tell_ended = |verdict: RunVerdict, tally: Option<&Tally>| {
    if let Some(id) = test_run {
      ask.witness.ended(
        id,
        &RunEnded {
          verdict,
          passed: tally.map_or(0, |tally| tally.pass),
          failed: tally.map_or(0, |tally| tally.fail),
          skipped: tally.map_or(0, |tally| tally.skip),
          ended_at: now_ms(),
        },
      );
    }
  };
  let errors = out.try_clone().map_err(no_log)?;
  let env = &ask.turn.env;
  let widened = ask.language.widen(&ask.command, lane_width(ask.turn.lane, env, machine_threads()));
  let Some((program, args)) = widened.command.split_first() else {
    tell_ended(RunVerdict::NotRun, None);
    return Ok(not_run());
  };
  // A process group of its own, so what the command started stands together under it.
  let mut child = Command::new(program)
    .args(args)
    .current_dir(&ask.cwd)
    .envs(&widened.env)
    .envs(turn.held_variable())
    .stdin(Stdio::null())
    .stdout(out)
    .stderr(errors)
    .process_group(0)
    .spawn()
    .map_err(|why| {
      tell_ended(RunVerdict::Failed, None);
      fill_marks(&said.unstarted, &[("program", program), ("why", &why.to_string())])
    })?;
  let status = child.wait().ok();
  let code = status.and_then(|status| status.code());
  let text = String::from_utf8_lossy(&std::fs::read(&log).unwrap_or_default()).into_owned();
  let ok = code == Some(0);
  let seconds = started.elapsed().as_secs_f64();

  record_turn_times(env, &ask.turn.command, ask.turn.lane, turn.waited_seconds(), seconds);

  let read = if ok { Vec::new() } else { failures_of_log(ask.languages, &text) };
  let mut failures = if read.is_empty() && !ok { PLAIN.failures(&text) } else { read };

  failures.truncate(LANES.numbers.failure_lines);

  let tally = tally_of_log(ask.languages, &text);

  tell_ended(if ok { RunVerdict::Passed } else { RunVerdict::Failed }, tally.as_ref());

  Ok(TestbedResult {
    language: ask.language.name(),
    ran: true,
    ok,
    code,
    tally,
    seconds,
    waited_seconds: turn.waited_seconds(),
    log: Some(log),
    failures,
    turn: turn.words(),
    test_run,
  })
}
