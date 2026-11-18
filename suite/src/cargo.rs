//! Cargo run over the Rust workspace the way every verb of the testbed runs it (docs/deploy.md, 「The Cargo
//! build directory」; docs/boundary.md, the `cargo` verb; docs/testbed.md): through the shared build
//! directory with the checkout's own crates apart in it, `clippy` as a check through the checkout's own clippy
//! wrapper, on the account's own toolchain, and — where it compiles — in the machine's turn behind the other
//! heavy commands on this machine, run again where the kernel killed it for want of memory, and ended as hung
//! where its standard output and error are both files and nothing is written to them for the silence limit.
//! Formatting and the other reading verbs compile nothing and run at once. A command holding its lock runs
//! with the lock's path packages brought to their manifests, the lock put back once it ends. A compiling
//! command is handed its turn's share of the lane's width as its jobs, unless its words or its environment name them. A build that
//! passed removes, inside the same turn, what the checkout's earlier build of its shape left.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use bmo_testbed_core::cargo_lock::{Aligned, align_path_packages};
use bmo_testbed_core::languages::{Cargo, Language as _, cargo_compiles as compiles};
use bmo_testbed_core::memory::{MachineHands, MemoryLimits, RunEnd, run_in_turn};
use bmo_testbed_core::policy::fill_shown;
use bmo_testbed_core::policy::suite::SUITE;
use bmo_testbed_core::silence::{Output, Silence, Watch};
use bmo_testbed_core::turn::{Env, Lane, TestbedTurn, lane_width, machine_threads, process_env};

use crate::crates::{
  cargo_environment, cargo_program, checkout_target_dir, on_path, put_in_place, set_by_cargo,
};
use crate::own_binary::testbed_line;
use crate::superseded::{Builds, remove_superseded, reporting_args};

// How `cargo clippy` hands its driver the lint arguments: each one followed by this mark, in one variable the
// driver reads and Cargo tracks.
const CLIPPY_ARGS_MARK: &str = "__CLIPPY_HACKERY__";

/// A Cargo command to run in a checkout: the program and its arguments, and the variables it is handed over
/// the caller's own, with every variable a Cargo running the caller set for its own package taken off.
#[derive(Clone, Debug)]
pub struct CargoInvocation {
  pub command: Vec<String>,
  pub env: Vec<(String, String)>,
}

impl CargoInvocation {
  /// The command ready to start in `root`, its standard input closed.
  pub fn command(&self, root: &Path) -> Command {
    let mut command = self.program();

    command.args(&self.command[1..]).current_dir(root).stdin(Stdio::null());
    command
  }

  /// Its program with its variables and no arguments, for another Cargo command asked in the same checkout.
  pub fn program(&self) -> Command {
    let mut command = Command::new(&self.command[0]);

    for (name, _) in std::env::vars_os() {
      if set_by_cargo(&name.to_string_lossy()) {
        command.env_remove(name);
      }
    }

    command.envs(self.env.iter().map(|(name, value)| (name, value)));
    command
  }

  /// The value it hands a variable, the last one named winning.
  pub fn named(&self, variable: &str) -> Option<&str> {
    self
      .env
      .iter()
      .rev()
      .find(|(name, _)| name == variable)
      .map(|(_, value)| value.as_str())
  }
}

// Clippy's wrapper: a link named `clippy-driver` in the checkout's own target directory, the name the toolchain
// installer's proxy in the Cargo home reads to run the driver of the toolchain Cargo runs on; the PATH's driver
// where there is no proxy. Its path, which differs from one checkout to the next, keeps each checkout's lint
// outputs apart.
fn clippy_wrapper(root: &Path, cargo_home: &str) -> PathBuf {
  let proxy = Path::new(cargo_home).join("bin").join("clippy-driver");
  let driver = if proxy.exists() {
    proxy
  } else {
    on_path("clippy-driver").unwrap_or_else(|| PathBuf::from("clippy-driver"))
  };
  let link = checkout_target_dir(root).join("workspace-clippy").join("clippy-driver");

  if std::fs::read_link(&link).ok().as_deref() != Some(driver.as_path()) {
    put_in_place(&link, |made| std::os::unix::fs::symlink(&driver, made));
  }

  link
}

/// A Cargo command as the verbs run it in the checkout at `root`. `clippy` runs as `cargo check` with the
/// checkout's own clippy wrapper, since `cargo clippy` would name its own driver, one path in every checkout,
/// and its lint arguments after `--` reach the driver the way `cargo clippy` hands them. What stands before
/// `--` goes to `cargo check` as it stands, so `cargo clippy`'s own options, such as `--fix`, are refused
/// there by Cargo.
pub fn cargo_invocation(root: &Path, args: &[String]) -> CargoInvocation {
  let mut env = cargo_environment(root);

  env.extend(build_cache(&process_env(), sccache_on_path().as_deref()));
  let cargo_home = env
    .iter()
    .find(|(name, _)| name == "CARGO_HOME")
    .map(|(_, home)| home.clone())
    .unwrap_or_default();
  let program = cargo_program(&cargo_home);

  if args.first().map(String::as_str) != Some("clippy") {
    return CargoInvocation {
      command: std::iter::once(program).chain(args.iter().cloned()).collect(),
      env,
    };
  }

  let split = args.iter().position(|arg| arg == "--").unwrap_or(args.len());
  let lint: String = args.iter().skip(split + 1).map(|arg| format!("{arg}{CLIPPY_ARGS_MARK}")).collect();

  env.retain(|(name, _)| name != "RUSTC_WORKSPACE_WRAPPER");
  env.push((
    "RUSTC_WORKSPACE_WRAPPER".to_owned(),
    clippy_wrapper(root, &cargo_home).to_string_lossy().into_owned(),
  ));
  env.push(("CLIPPY_ARGS".to_owned(), lint));

  CargoInvocation {
    command: [program, "check".to_owned()]
      .into_iter()
      .chain(args[1..split].iter().cloned())
      .collect(),
    env,
  }
}

/// Cargo's variables that set incremental state for the development, the test and the release profile, over
/// what a manifest's profiles say. The one variable that sets it for every profile is left alone: the compile
/// cache refuses to run under it.
pub const INCREMENTAL_VARIABLES: [&str; 3] = [
  "CARGO_PROFILE_DEV_INCREMENTAL",
  "CARGO_PROFILE_TEST_INCREMENTAL",
  "CARGO_PROFILE_RELEASE_INCREMENTAL",
];

/// Cargo's variable naming the program every compiler call goes through.
pub const WRAPPER_VARIABLE: &str = "RUSTC_WRAPPER";

const CACHE_PROGRAM: &str = "sccache";

/// What a build is given so that it compiles as little as it must, as the Cargo Book states both: the
/// incremental state its profiles' `incremental` setting keeps, set for each profile over the manifest's own
/// word, and the compile cache its 「Shared cache」 names, where the machine has it at `cache`, so that a
/// dependency compiled in one checkout is not compiled again in another. A caller's own setting of a
/// variable is kept.
pub fn build_cache(env: &Env, cache: Option<&Path>) -> Vec<(String, String)> {
  let mut set: Vec<(String, String)> = INCREMENTAL_VARIABLES
    .iter()
    .filter(|name| !env.contains_key(**name))
    .map(|name| ((*name).to_owned(), "true".to_owned()))
    .collect();

  if let Some(cache) = cache.filter(|_| !env.contains_key(WRAPPER_VARIABLE)) {
    set.push((WRAPPER_VARIABLE.to_owned(), cache.to_string_lossy().into_owned()));
  }

  set
}

// The compile cache's program, where the search path holds one.
fn sccache_on_path() -> Option<PathBuf> {
  let path = std::env::var_os("PATH")?;

  std::env::split_paths(&path)
    .map(|dir| dir.join(CACHE_PROGRAM))
    .find(|candidate| candidate.is_file())
}

/// The lock of the checkout's workspace with its path packages brought to their manifests, held so for as long
/// as the value lives and put back as it was found once it goes, unless something else has written it since:
/// for a run whose Cargo commands holding the lock do not all go through the verb.
pub struct HeldLock(Aligned);

impl HeldLock {
  /// The lock at `root` aligned for the Cargo command `args` and held until the value goes; none where the
  /// checkout holds no workspace manifest of its own, the command does not hold its lock, or the lock needed
  /// nothing or could not be aligned (`align_path_packages`).
  pub fn aligned(root: &Path, args: &[String]) -> Option<Self> {
    if !root.join("Cargo.toml").is_file() {
      return None;
    }

    let invocation = cargo_invocation(root, args);

    align_path_packages(root, args, &|| invocation.program()).map(Self)
  }

  /// The one line that says which path packages moved.
  pub fn said(&self) -> String {
    self.0.said()
  }
}

impl Drop for HeldLock {
  fn drop(&mut self) {
    self.0.put_back();
  }
}

/// The words running the Cargo verb with `args` as a process of its own, through the running
/// `bmo-testbed`, which carries it: for a step whose output, environment and checkout are its own, which
/// reaches the verb's turn, its retry and its removal of what a build superseded as `bmo-testbed cargo` does.
pub fn cargo_verb_line(args: &[impl AsRef<str>]) -> Vec<String> {
  testbed_line(&std::iter::once("cargo").chain(args.iter().map(AsRef::as_ref)).collect::<Vec<_>>())
}

/// The removal after a passed Cargo command `args` of the verbs, whose output went to a person: Cargo is asked
/// again with the same words for what the build stood on, building and running nothing more.
pub fn remove_superseded_after(root: &Path, args: &[String], say: &mut dyn FnMut(&str)) {
  let Some(reporting) = reporting_args(args) else { return };
  let invocation = cargo_invocation(root, &reporting);
  // The command asked about ran on the lock's path packages brought to their manifests and put the lock back,
  // so the asking holds the lock the same way or Cargo refuses the lock the build itself passed on.
  let aligned = align_path_packages(root, &reporting, &|| invocation.program());
  let asked = invocation.command(root).stdout(Stdio::piped()).stderr(Stdio::piped()).output();

  if let Some(aligned) = &aligned {
    aligned.put_back();
  }

  let asked = match asked {
    Ok(asked) => asked,
    Err(why) => return say(&fill_shown(&SUITE.cargo.not_removed, &[("why", &why)])),
  };

  if !asked.status.success() {
    let stderr = String::from_utf8_lossy(&asked.stderr);
    let said = stderr.trim().lines().last().unwrap_or_default();
    let code = asked.status.code().map_or_else(|| SUITE.cargo.no_code.clone(), |code| code.to_string());
    let why = fill_shown(&SUITE.cargo.unasked, &[("code", &code), ("said", &said)]);

    return say(&fill_shown(&SUITE.cargo.not_removed, &[("why", &why)]));
  }

  let wrapper = invocation.named("RUSTC_WORKSPACE_WRAPPER").unwrap_or_default();

  remove_superseded(&Builds::of(root), wrapper, &String::from_utf8_lossy(&asked.stdout), say);
}

/// Cargo with `args` in the checkout at `root`, answering the exit code the caller ends on: a command that
/// compiles nothing at once, one that compiles in the machine's turn, watched for silence and run again after
/// an out-of-memory kill, each start of one holding its lock run on the lock's path packages brought to their
/// manifests, with what it superseded removed once it has passed. Cargo stays in this process's
/// own process group, so a terminal's interrupt still reaches it; a hung one is ended with every process
/// under it.
pub fn run_cargo(root: &Path, args: &[String]) -> u8 {
  let invocation = cargo_invocation(root, args);

  let compiling = compiles(args);
  let what = std::iter::once("cargo")
    .chain(args.iter().map(String::as_str))
    .collect::<Vec<_>>()
    .join(" ");
  let env: Env = process_env();
  let silence = Silence::at(root);
  let start = |turn: Option<&TestbedTurn>| {
    let output = if compiling { Output::own() } else { Output::Files(Vec::new()) };
    let watch = Watch::new(output, silence);
    let mut command = invocation.command(root);
    // The turn's share of the lane, as Cargo's jobs, where neither the words nor a caller already name them.
    let widened = turn
      .filter(|_| !env.contains_key("CARGO_BUILD_JOBS") && invocation.named("CARGO_BUILD_JOBS").is_none())
      .map(|turn| {
        let width = turn.width(lane_width(Lane::Wide, &env, machine_threads()));

        Cargo.widen(&invocation.command, width).env
      })
      .unwrap_or_default();

    command
      .envs(turn.and_then(TestbedTurn::held_variable))
      .envs(widened)
      .envs(watch.variable());

    let aligned = align_path_packages(root, args, &|| invocation.program());

    if let Some(aligned) = &aligned {
      eprintln!("{}", aligned.said());
    }

    let ended = match command.spawn() {
      Ok(mut child) => {
        let waited = watch.wait(&mut child, &what);

        if let Some(hung) = &waited.hung {
          eprintln!("{hung}");
        }

        RunEnd::from(&waited)
      }
      Err(why) => {
        eprintln!(
          "{}",
          fill_shown(&SUITE.cargo.unstarted, &[("command", &invocation.command[0]), ("why", &why)])
        );

        RunEnd {
          code: Some(127),
          signal: None,
        }
      }
    };

    if let Some(aligned) = &aligned {
      aligned.put_back();
    }

    ended
  };

  if !compiling {
    return start(None).exit_code();
  }

  let mut start_in = |turn: &TestbedTurn| start(Some(turn));
  let mut hands = MachineHands::new(
    format!("cargo {}", args[0]),
    Lane::Wide,
    root.to_string_lossy().into_owned(),
    env.clone(),
    &mut start_in,
  );

  run_in_turn(&mut hands, &MemoryLimits::default(), &mut || {
    remove_superseded_after(root, args, &mut |line| eprintln!("{line}"));
  })
}
