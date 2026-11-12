//! The static check that goes before every run of selected tests, and where such a run begins
//! (docs/checks.md, 「The static check」 and 「Which tests a change runs」). The change check and a
//! deployment's static check each ask here, so the two cannot check differently.

use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use bmo_testbed_core::declaration::{self, Repository};
use bmo_testbed_core::memory::{MachineHands, MemoryLimits, RunEnd, run_in_turn};
use bmo_testbed_core::policy::deploy_steps::DeployStep;
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::static_check::STATIC_CHECK;
use bmo_testbed_core::turn::{
  Lane, NOT_RUN_EXIT_CODE, TURN_TIMES_VARIABLE, process_env, read_turn_times, times_words,
};

use crate::cargo::cargo_verb_line;
use crate::crates::{CrateDeclaration, workspace_crates};
use crate::own_binary::testbed_line;
use crate::selection::{Layout, changed_paths, glob_matches, select_crates};

/// One command of the check, run in a directory of the tree being checked, with `env` over the caller's: in
/// the lane it names, run again after an out-of-memory kill where it asks to be, or at once where it names
/// none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckStep {
  pub id: String,
  pub dir: PathBuf,
  pub command: Vec<String>,
  pub env: Vec<(String, String)>,
  pub lane: Option<Lane>,
  pub retry_out_of_memory: bool,
  /// The Cargo workspace check's own step a declared step only repeats, where that check runs: the step is
  /// then left out, said in one line and counted as passed.
  pub covered_by: Option<DeployStep>,
}

impl CheckStep {
  // A step of the testbed's own at the root, run where it stands: the verbs it runs take their own turns.
  fn own(step: DeployStep, command: Vec<String>) -> Self {
    Self {
      id: step.name().to_owned(),
      dir: PathBuf::from("."),
      command,
      env: Vec::new(),
      lane: None,
      retry_out_of_memory: false,
      covered_by: None,
    }
  }

  /// A step the repository declares, its command one line run through `sh` in the directory it names.
  pub fn declared(step: &declaration::CheckStep) -> Self {
    Self {
      id: step.id.clone(),
      dir: step.dir.clone().unwrap_or_else(|| PathBuf::from(".")),
      command: vec!["sh".to_owned(), "-c".to_owned(), step.command.clone()],
      env: Vec::new(),
      lane: step.lane,
      retry_out_of_memory: step.retry_out_of_memory,
      covered_by: None,
    }
  }
}

/// The quick checks the repository enables, read in about a second, by the running `bmo-testbed`'s `check
/// quick`.
pub fn quick_check() -> CheckStep {
  CheckStep::own(DeployStep::Quick, testbed_line(&["check", "quick"]))
}

/// The packages of a Cargo workspace its formatting and lints judge: the repository's own, none of a sibling
/// repository's (docs/checks.md, 「The static check」).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OwnPackages {
  /// The whole workspace, where every member stands inside the repository's root, or where Cargo cannot read
  /// the workspace.
  Workspace,
  /// The members inside the root, by name, where a member stands outside it.
  Named(Vec<String>),
}

impl OwnPackages {
  /// The packages of the workspace at `root`, as Cargo reads its members.
  pub fn of(root: &Path) -> Self {
    workspace_crates(root).map_or(Self::Workspace, |crates| Self::among(&crates))
  }

  /// The packages among a workspace's crates, a crate outside the root named by its whole path.
  pub fn among(crates: &[CrateDeclaration]) -> Self {
    if crates.iter().all(|one| !Path::new(&one.dir).is_absolute()) {
      return Self::Workspace;
    }

    Self::Named(
      crates
        .iter()
        .filter(|one| !Path::new(&one.dir).is_absolute())
        .map(|one| one.name.clone())
        .collect(),
    )
  }

  // Cargo's words selecting them, `whole` the words for the whole workspace.
  fn args(&self, whole: &[&str]) -> Vec<String> {
    match self {
      Self::Workspace => whole.iter().map(|&word| word.to_owned()).collect(),
      Self::Named(names) => names.iter().flat_map(|name| ["-p".to_owned(), name.clone()]).collect(),
    }
  }

  /// Whether there is anything of the repository's own to judge.
  pub fn any(&self) -> bool {
    !matches!(self, Self::Named(names) if names.is_empty())
  }
}

/// The formatting of the repository's own packages, through the Cargo verb, which runs it at once since it
/// compiles nothing. No path dependency is followed, so a sibling repository's files are never read.
pub fn cargo_format(own: &OwnPackages) -> CheckStep {
  let args: Vec<String> = ["fmt", "--check"].into_iter().map(str::to_owned).chain(own.args(&[])).collect();

  CheckStep::own(DeployStep::CargoFormat, cargo_verb_line(&args))
}

/// The lints of the repository's own packages, which check every crate as they lint it, through the Cargo
/// verb, which runs them in the wide lane and again after an out-of-memory kill. A path dependency is
/// compiled as a dependency, and its warnings are never denied.
pub fn cargo_lint(own: &OwnPackages) -> CheckStep {
  let args: Vec<String> = std::iter::once("clippy".to_owned())
    .chain(own.args(&["--workspace"]))
    .chain(["--all-targets", "--locked", "--", "-D", "warnings"].into_iter().map(str::to_owned))
    .collect();

  CheckStep::own(DeployStep::CargoLint, cargo_verb_line(&args))
}

// The words of a Cargo formatting check the workspace's formatting covers, `--check` among them and needed
// before `--` or alone after it. They mirror what `cargo_format` judges, so the two change together.
const FORMAT_COVERS: [&str; 4] = ["--check", "--all", "-q", "--quiet"];

// The words of a Cargo lint or type check the workspace's lints cover. They mirror what `cargo_lint` judges.
const LINT_COVERS: [&str; 8] = [
  "--all",
  "--workspace",
  "--all-targets",
  "--locked",
  "--offline",
  "--frozen",
  "-q",
  "--quiet",
];

// The lint words after `--` the workspace's lints cover: every warning denied.
const DENIED_WARNINGS: [&[&str]; 2] = [&["-D", "warnings"], &["-Dwarnings"]];

/// The Cargo workspace check's own step a declared step only repeats: a Cargo command at the root, split the
/// way the shell splits it, checking the formatting, the lints or the types with no word the workspace check
/// does not already judge (docs/checks.md, 「The static check」). None where the step judges anything more,
/// and then it runs as declared.
pub fn covered_by(step: &declaration::CheckStep) -> Option<DeployStep> {
  let at_root = step
    .dir
    .as_deref()
    .is_none_or(|dir| dir.components().all(|part| part == Component::CurDir));
  let words = shlex::split(&step.command).filter(|_| at_root)?;
  let words: Vec<&str> = words.iter().map(String::as_str).collect();
  let (subcommand, rest) = match words.as_slice() {
    ["cargo", subcommand, rest @ ..] | ["bmo-testbed", "cargo", subcommand, rest @ ..] => {
      (*subcommand, rest)
    }
    _ => return None,
  };
  let (args, after) = match rest.iter().position(|&word| word == "--") {
    Some(at) => (&rest[..at], Some(&rest[at + 1..])),
    None => (rest, None),
  };
  let only = |allowed: &[&str]| args.iter().all(|word| allowed.contains(word));

  match (subcommand, after) {
    ("fmt", None) if args.contains(&"--check") && only(&FORMAT_COVERS) => Some(DeployStep::CargoFormat),
    ("fmt", Some(["--check"])) if only(&FORMAT_COVERS) => Some(DeployStep::CargoFormat),
    ("clippy", Some(lints)) if DENIED_WARNINGS.contains(&lints) && only(&LINT_COVERS) => {
      Some(DeployStep::CargoLint)
    }
    ("clippy" | "check", None) if only(&LINT_COVERS) => Some(DeployStep::CargoLint),
    _ => None,
  }
}

// What a change from `since` changed; none where every path is reached: a run with no range, or a range the
// checkout cannot read.
fn changed_since(root: &Path, since: Option<&str>) -> Option<Vec<String>> {
  changed_paths(root, since?).ok()
}

/// Whether a change of `changed` marks any crate of the Cargo workspace. A checkout with no workspace marks
/// none; every path changed, and a workspace that cannot be read, mark them all.
pub fn cargo_selected(repo: &Repository, changed: Option<&[String]>) -> bool {
  if !repo.root.join("Cargo.toml").exists() {
    return false;
  }

  let Some(changed) = changed else { return true };

  match workspace_crates(&repo.root) {
    Ok(crates) => !select_crates(&Layout::of_parts(repo, crates), changed).is_empty(),
    Err(_) => true,
  }
}

/// Whether a declared step's `when` globs reach a change of `changed`: a step declaring none is reached by
/// every change, and every path changed reaches every step.
pub fn step_reached(step: &declaration::CheckStep, changed: Option<&[String]>) -> bool {
  let (Some(when), Some(changed)) = (&step.when, changed) else { return true };

  when.iter().any(|glob| changed.iter().any(|path| glob_matches(glob, path)))
}

/// The steps of the repository's own a change of `changed` owes, every path where none is named: the quick
/// checks where it enables any, then its declared steps the change reaches, in their order, each run as
/// declared.
pub fn declared_steps(repo: &Repository, changed: Option<&[String]>) -> Vec<CheckStep> {
  own_steps(repo, changed, false)
}

// The repository's own steps, each declared one that only repeats the Cargo workspace check marked as covered
// where `workspace_checked`.
fn own_steps(repo: &Repository, changed: Option<&[String]>, workspace_checked: bool) -> Vec<CheckStep> {
  let check = &repo.declaration.check;
  let quick = (!check.quick.is_empty()).then(quick_check);
  let declared = check.steps.iter().filter(|step| step_reached(step, changed)).map(|step| CheckStep {
    covered_by: covered_by(step).filter(|_| workspace_checked),
    ..CheckStep::declared(step)
  });

  quick.into_iter().chain(declared).collect()
}

/// The steps a change from `since` owes: the repository's own, then the formatting and lints of the Cargo
/// workspace's own packages where a crate is marked, a declared step that only repeats them left out
/// (docs/checks.md, 「The static check」).
pub fn check_steps(repo: &Repository, since: Option<&str>) -> Vec<CheckStep> {
  let changed = changed_since(&repo.root, since);
  let changed = changed.as_deref();
  let own = cargo_selected(repo, changed)
    .then(|| OwnPackages::of(&repo.root))
    .filter(OwnPackages::any);
  let mut steps = own_steps(repo, changed, own.is_some());

  if let Some(own) = own {
    steps.extend([cargo_format(&own), cargo_lint(&own)]);
  }

  steps
}

/// What a command run with its waits read back came to.
#[derive(Clone, Debug, PartialEq)]
pub struct TimedRun {
  pub code: i32,
  pub output: String,
  pub waited_seconds: f64,
  pub ran_seconds: f64,
}

// A run of `run`, handed the file each heavy command under it writes its wait for the machine's turn into,
// answering its exit code and output: the waits summed are the run's wait, the rest its run.
fn timed(run: impl FnOnce(&Path) -> (i32, String)) -> TimedRun {
  let times = std::env::temp_dir().join(format!(
    "bmo-testbed-timed-run-{}-{}.jsonl",
    std::process::id(),
    run_number()
  ));
  let started = Instant::now();
  let (code, output) = run(&times);
  let seconds = started.elapsed().as_secs_f64();
  let waited = read_turn_times(&times).iter().map(|one| one.waited_seconds).sum::<f64>().min(seconds);

  let _ = std::fs::remove_file(&times);

  TimedRun {
    code,
    output,
    waited_seconds: waited,
    ran_seconds: seconds - waited,
  }
}

// A command started with its output taken, `times` the file its heavy commands write their waits into.
fn output_of(
  command: &[String],
  cwd: &Path,
  env: &[(String, String)],
  times: &Path,
  held: Option<(&'static str, String)>,
) -> (RunEnd, String) {
  // The output goes to files of the run's own and never a pipe: a process the command leaves running keeps
  // a pipe open, and a reader of it would wait for that process long after the command ended.
  let said = times.with_extension("out");
  let failed = times.with_extension("err");
  let done = std::fs::File::create(&said)
    .and_then(|out| Ok((out, std::fs::File::create(&failed)?)))
    .and_then(|(out, errors)| {
      Command::new(&command[0])
        .args(&command[1..])
        .current_dir(cwd)
        .envs(env.iter().map(|(name, value)| (name, value)))
        .envs(held)
        .env(TURN_TIMES_VARIABLE, times)
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(errors)
        .status()
    });
  let taken = |path: &Path| String::from_utf8_lossy(&std::fs::read(path).unwrap_or_default()).into_owned();
  let output = format!("{}{}", taken(&said), taken(&failed));

  let _ = std::fs::remove_file(&said);
  let _ = std::fs::remove_file(&failed);

  match done {
    Ok(status) => (RunEnd::from(status), output),
    Err(why) => (
      RunEnd {
        code: Some(127),
        signal: None,
      },
      why.to_string(),
    ),
  }
}

/// A command run to its end where it stands with its output taken, each heavy command under it writing its
/// wait for the machine's turn into a file of the run's own.
pub fn run_timed(command: &[String], cwd: &Path, env: &[(String, String)]) -> TimedRun {
  timed(|times| {
    let (end, output) = output_of(command, cwd, env, times, None);

    (i32::from(end.exit_code()), output)
  })
}

// A name no two runs of this process share.
fn run_number() -> u64 {
  use std::sync::atomic::{AtomicU64, Ordering};

  static RUNS: AtomicU64 = AtomicU64::new(0);

  RUNS.fetch_add(1, Ordering::Relaxed)
}

/// One step done, as a caller writing down where its check stands is told it.
#[derive(Clone, Debug, PartialEq)]
pub struct StepDone {
  pub id: String,
  pub code: i32,
  pub waited_seconds: f64,
  pub ran_seconds: f64,
}

/// A step as the check's lines name it.
pub fn step_words(step: &CheckStep) -> String {
  fill_marks(
    &STATIC_CHECK.step,
    &[
      ("name", &step.id),
      ("command", &step.command.join(" ")),
      ("dir", &step.dir.to_string_lossy()),
    ],
  )
}

/// The steps in order, each one's output handed to `write` as it ends with how long it waited for the
/// machine's turn and how long it ran, each told to `told`; a step the Cargo workspace check covers is not
/// run, its one line handed to `write` and told as passed with no wait and no run. Every step runs to its
/// end, a failing one stopping none after it; the answer is the line naming the first that failed or did
/// not get its turn, a step not run named before a failed one since running again is all it needs, and none
/// where every step passed.
pub fn run_static_check(
  root: &Path,
  steps: &[CheckStep],
  write: &mut dyn FnMut(&str),
  run: &dyn Fn(&Path, &CheckStep) -> TimedRun,
  told: &mut dyn FnMut(&StepDone),
) -> Option<String> {
  let said = &*STATIC_CHECK;
  let mut not_run: Option<String> = None;
  let mut failed: Option<String> = None;

  for step in steps {
    if let Some(by) = step.covered_by {
      told(&StepDone {
        id: step.id.clone(),
        code: 0,
        waited_seconds: 0.0,
        ran_seconds: 0.0,
      });
      write(&format!(
        "{}\n",
        fill_marks(&said.left_out, &[("step", &step_words(step)), ("by", by.name())])
      ));
      continue;
    }

    let started = Instant::now();
    let result = run(root, step);
    let seconds = started.elapsed().as_secs_f64();
    let waited = result.waited_seconds.min(seconds);

    told(&StepDone {
      id: step.id.clone(),
      code: result.code,
      waited_seconds: waited,
      ran_seconds: seconds - waited,
    });

    let newline = if result.output.is_empty() || result.output.ends_with('\n') { "" } else { "\n" };

    write(&format!(
      "{}\n{}{newline}",
      fill_marks(
        &said.ended,
        &[
          ("step", &step_words(step)),
          ("code", &result.code.to_string()),
          ("times", &times_words(waited, seconds - waited))
        ],
      ),
      result.output
    ));

    if result.code == i32::from(NOT_RUN_EXIT_CODE) {
      not_run.get_or_insert_with(|| {
        fill_marks(&said.not_had, &[("mark", &said.not_run), ("step", &step_words(step))])
      });
    } else if result.code != 0 {
      failed.get_or_insert_with(|| {
        fill_marks(&said.failed, &[("step", &step_words(step)), ("code", &result.code.to_string())])
      });
    }
  }

  failed.or(not_run)
}

/// A step run in the tree, its heavy commands' waits read back as the step's wait: where it stands, or in
/// its lane's turn, run again after an out-of-memory kill where it asks to be.
pub fn spawn_step(root: &Path, step: &CheckStep) -> TimedRun {
  let cwd = root.join(&step.dir);
  let Some(lane) = step.lane else { return run_timed(&step.command, &cwd, &step.env) };

  timed(|times| {
    let mut output = String::new();
    let mut start = |held: Option<(&'static str, String)>| {
      let (end, said) = output_of(&step.command, &cwd, &step.env, times, held);

      output.push_str(&said);
      end
    };
    let mut env = process_env();

    env.insert(TURN_TIMES_VARIABLE.to_owned(), times.to_string_lossy().into_owned());

    let limits = MemoryLimits {
      runs: if step.retry_out_of_memory { MemoryLimits::default().runs } else { 1 },
      ..MemoryLimits::default()
    };
    let mut hands =
      MachineHands::new(step.id.clone(), lane, root.to_string_lossy().into_owned(), env, &mut start);
    let code = run_in_turn(&mut hands, &limits, &mut || {});

    drop(hands);
    (i32::from(code), output)
  })
}

/// The newest of these commits HEAD descends from: the one with the fewest commits between it and HEAD. A
/// commit HEAD does not descend from, or one the checkout cannot read, is passed over; none where none is
/// left, and then the run is whole.
pub fn newest_vetted(root: &Path, candidates: &[Option<String>]) -> Option<String> {
  let git = |args: &[&str]| {
    Command::new("git")
      .args(args)
      .current_dir(root)
      .stdin(Stdio::null())
      .stderr(Stdio::null())
      .output()
      .ok()
      .filter(|done| done.status.success())
      .map(|done| String::from_utf8_lossy(&done.stdout).trim().to_owned())
  };
  let mut best: Option<(String, u64)> = None;
  let mut seen: Vec<&str> = Vec::new();

  for commit in candidates.iter().flatten().filter(|commit| !commit.is_empty()) {
    if seen.contains(&commit.as_str()) {
      continue;
    }
    seen.push(commit);

    if git(&["merge-base", "--is-ancestor", commit, "HEAD"]).is_none() {
      continue;
    }

    let Some(distance) =
      git(&["rev-list", "--count", &format!("{commit}..HEAD")]).and_then(|out| out.parse().ok())
    else {
      continue;
    };

    if best.as_ref().is_none_or(|(_, nearest)| distance < *nearest) {
      best = Some((commit.clone(), distance));
    }
  }

  best.map(|(commit, _)| commit)
}
