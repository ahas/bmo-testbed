//! A run placed elsewhere (docs/boundary.md, 「Placement」). Through the placement endpoint, against a
//! stand-in endpoint the test serves: an answer of local, or one that does not come or does not read, runs
//! here; an answer of remote makes the bench ready over a stand-in `ssh` that runs its line on this machine,
//! and the run's output and result come back off its stream. On the placed machine the settings name: the
//! bench made ready over the same stand-in, from the remote that machine can reach or empty where it can reach
//! none, the run the testbed's `placed` verb there, a machine that cannot be reached, or settings that do not
//! read, leaving the run here, the shared versions laid over their copy as their main branch holds them and a
//! placed check from a copy of the shared repository reading them there, a
//! deployment's bench standing apart from a run's copy of the same tree, its sweep keeping a laid sibling that
//! shares the deployed repository's root commits while the bench another checkout left goes and the making of
//! the bench blaming itself naming the tree where a laid sibling does not stand, a testbed there that is not the
//! live testbed release refused before any bench is made, brought to the live release read at each asking, and a
//! run meeting one that cannot be brought ending as not run with its event, a machine out of reach waited for and a
//! run ending as not run with its event once the wait is spent, never running here, one event for each outage and one
//! for the machine's return, sent to the hook the machine's testbed environment file names, the deployment's gate and build
//! answering broken for it whether or not they began there, `status` reading the machine's release against the live
//! one, a run whose environment sets a variable its declaration names as this machine's kept here, a run from a
//! folder below the checkout's top running in that folder there and one from a folder the bench copy leaves out
//! kept here, the copy carrying what git sees in the tree, a file a `!` line takes back among them, the copy
//! going by content so a file whose time is older than the bench's last build is built there again, an outside
//! tree no repository holds refused, the `placed` verb refusing a folder the bench does not hold, the
//! run standing in its lane here while it lasts, running once the far side says its turn came, that line read
//! and not passed on, and the `placed` verb there passing its command's output on, telling its own holder
//! while it comes and ending it whole once it falls silent for the declared limit. The units a release marks
//! for the machine laid in its user units and their timers started there, and the prune's reading of its free
//! disk against the settings' floor, sending `placed-disk-low` for each disk below it. The settings' order read
//! by name, and a run going to the first machine in it with a seat of its lane free, falling to the next when
//! that one is full. The testbed there brought to
//! a live release carrying one and read back at its commit, unless the caller keeps it as it stands.

use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::response::sse::{Event, Sse};
use axum::routing::{get, post};
use bmo_testbed_cli::cargo::placed_disk;
use bmo_testbed_cli::deploy::{PlacedGate, brings_testbed};
use bmo_testbed_cli::placement::{
  BenchFor, MACHINE_ENV_FILE, MachinePlacement, MachineSettings, MachineState, Ordered, PlacementState,
  ReadyBench, STANDING_VARIABLE, machine_hook, reachable_remote,
};
use bmo_testbed_core::declaration::Declaration;
use bmo_testbed_core::events::Hook;
use bmo_testbed_core::placement::{
  LOADED_EXIT_CODE, LOADED_VARIABLE, PlaceAsk, PlacedRun, Placement, read_placed,
};
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::lanes::LANES;
use bmo_testbed_core::policy::prune::PRUNE;
use bmo_testbed_core::policy::suite::SUITE;
use bmo_testbed_core::policy::verbs::VERBS;
use bmo_testbed_core::queue::process_alive;
use bmo_testbed_core::turn::{Env, Lane, NOT_RUN_EXIT_CODE, TestbedTurn, TurnAsk, lock_path, take_turn};
use bmo_testbed_deploy::deployment::{BuildAsk, Gate, GateAsk, GateFault, Passed, none_earlier};
use futures_util::StreamExt as _;
use serde_json::{Value, json};

use crate::stand::{Ran, ReceiverStand, checkout, git, install, ran, testbed};

// A stand-in endpoint: what it was asked, and the address it serves on, which ends with the runtime.
struct Endpoint {
  _runtime: tokio::runtime::Runtime,
  base: String,
  asked: Arc<Mutex<Vec<Value>>>,
}

// An endpoint answering `answer` to every run, `{base}` in it standing for its own address, and serving the
// run's stream as the events given.
fn endpoint(answer: Value, events: Vec<(&'static str, String)>) -> Endpoint {
  endpoint_quiet(answer, events, Duration::ZERO)
}

// An endpoint as `endpoint` is, whose run's stream stays quiet for `quiet` before its events, as a run there
// whose command writes to its log does.
fn endpoint_quiet(answer: Value, events: Vec<(&'static str, String)>, quiet: Duration) -> Endpoint {
  let runtime = tokio::runtime::Builder::new_multi_thread()
    .worker_threads(1)
    .enable_all()
    .build()
    .unwrap();
  let listener = runtime.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
  let base = format!("http://{}", listener.local_addr().unwrap());
  let asked = Arc::new(Mutex::new(Vec::new()));
  let answer: Value = serde_json::from_str(&answer.to_string().replace("{base}", &base)).unwrap();
  let keeping = Arc::clone(&asked);
  let app = Router::new()
    .route(
      "/place",
      post(move |axum::Json(body): axum::Json<Value>| {
        keeping.lock().unwrap().push(body);
        let answer = answer.clone();

        async move { axum::Json(answer) }
      }),
    )
    .route(
      "/run",
      get(move || {
        let events = events.clone();

        async move {
          let quiet = futures_util::stream::once(tokio::time::sleep(quiet))
            .filter_map(|()| std::future::ready(None::<Result<Event, Infallible>>));

          Sse::new(
            quiet.chain(futures_util::stream::iter(
              events
                .into_iter()
                .map(|(name, data)| Ok::<_, Infallible>(Event::default().event(name).data(data))),
            )),
          )
        }
      }),
    );

  runtime.spawn(async move { axum::serve(listener, app).await });
  Endpoint {
    _runtime: runtime,
    base,
    asked,
  }
}

// A stand-in `ssh` that runs the line it is handed on this machine, past its options and its destination,
// the way git and rsync hand it theirs; the far side is a machine no load makes loaded, so the load of the
// machine running the tests decides nothing of where a run goes.
const SSH: &str = r#"#!/bin/sh
export BMO_TESTBED_LOADED_PERCENT=1000000
while [ $# -gt 0 ]; do
  case "$1" in
    -o|-p|-i|-l|-F) shift 2 ;;
    -*) shift ;;
    *) break ;;
  esac
done
shift
exec sh -c "$*"
"#;

// A run of the binary in a checkout, placed through the endpoint.
fn placed(repo: &Path, data: &Path, endpoint: &str, args: &[&str]) -> Ran {
  ran(testbed(repo, data, args).env("BMO_TESTBED_PLACEMENT", format!("{endpoint}/place")))
}

fn bench_answer(dir: &Path, ssh: &Path) -> Value {
  json!({
    "place": "remote",
    "machine": "machine-2",
    "bench": { "destination": "stand", "path": dir.join("bench"), "ssh": [ssh] },
    "run": "{base}/run",
  })
}

// "an answer of local runs the command here and says nothing of it, and the endpoint is told the run"
#[test]
fn an_answer_of_local_runs_here_and_the_endpoint_is_told_the_run() {
  let dir = tempfile::tempdir().unwrap();
  let repo = dir.path().join("repo");
  let local = endpoint(json!({ "place": "local" }), Vec::new());

  checkout(&repo, "testbed = 0\n");

  let done = placed(&repo, dir.path(), &local.base, &["run", "--narrow", "--", "sh", "-c", "exit 0"]);
  let asked = local.asked.lock().unwrap().clone();

  assert_eq!(done.code, 0, "{}", done.err);
  assert!(!done.err.contains("placement endpoint"), "{}", done.err);
  assert_eq!(asked.len(), 1);
  assert_eq!(asked[0]["command"], json!(["sh", "-c", "exit 0"]));
  assert_eq!(asked[0]["lane"], "narrow");
  assert!(asked[0].get("wait_seconds").is_none() && asked[0].get("guard_seconds").is_none());
  assert_eq!(asked[0]["commit"], git(&repo, &["rev-parse", "HEAD"]));
  assert_eq!(asked[0]["checkout"], git(&repo, &["rev-parse", "--show-toplevel"]));
  assert_eq!(asked[0]["remote"], "");
  assert_eq!((asked[0]["front"].clone(), asked[0]["next"].clone()), (json!(false), json!(false)));
  assert_eq!(asked[0]["directory"], "");

  // A run started in a folder below the top names the folder from the top.
  let folder = repo.join("studio/client");

  std::fs::create_dir_all(&folder).unwrap();
  placed(&folder, dir.path(), &local.base, &["run", "--", "sh", "-c", "exit 0"]);
  assert_eq!(local.asked.lock().unwrap()[1]["directory"], "studio/client");
}

// "an endpoint that does not answer, or answers what does not read, leaves the run here, said on a line"
#[test]
fn an_endpoint_that_does_not_answer_or_does_not_read_leaves_the_run_here() {
  let dir = tempfile::tempdir().unwrap();
  // A port nothing listens on, on the IPv6 loopback: the IPv4 one refuses nowhere a mirrored-network WSL machine
  // runs the suite.
  let gone = std::net::TcpListener::bind("[::1]:0").unwrap().local_addr().unwrap();
  let unread = endpoint(json!({ "place": "moon" }), Vec::new());
  let opening = fill_marks(&LANES.placed.local, &[("endpoint", "")]);
  let opening = opening.split("{why}").next().unwrap().split(" gave").next().unwrap();

  for base in [format!("http://{gone}"), unread.base.clone()] {
    let done = placed(dir.path(), dir.path(), &base, &["run", "--", "sh", "-c", "exit 0"]);

    assert_eq!(done.code, 0, "{base}: {}", done.err);
    assert!(done.err.contains(opening) && done.err.contains(&base), "{base}: {}", done.err);
  }
}

// "a run placed elsewhere runs in a bench copy of the checkout, and ends with the code its stream gives"
#[test]
fn a_run_placed_elsewhere_runs_in_a_bench_and_ends_with_the_code_its_stream_gives() {
  let dir = tempfile::tempdir().unwrap();
  let repo = dir.path().join("repo");
  let ssh = dir.path().join("ssh");
  let result = "testbed: FAILED in 3s (waited 0s) — cargo: 1 passed, 1 failed, 0 skipped; log: /x.log";
  let remote = endpoint(
    bench_answer(dir.path(), &ssh),
    vec![
      ("output", "remote says hi".to_owned()),
      ("result", json!({ "line": result, "code": 3 }).to_string()),
    ],
  );

  install(SSH, &ssh);
  checkout(&repo, "testbed = 0\n");
  std::fs::write(repo.join(".gitignore"), "ignored.txt\n").unwrap();
  std::fs::write(repo.join("ignored.txt"), "left out").unwrap();
  std::fs::write(repo.join("open.txt"), "not yet committed").unwrap();

  let done = placed(&repo, dir.path(), &remote.base, &["run", "--", "cargo", "test"]);
  let bench = dir.path().join("bench");
  let running =
    fill_marks(&LANES.placed.running, &[("machine", "machine-2"), ("bench", &bench.to_string_lossy())]);

  assert_eq!(done.code, 3, "{}\n{}", done.out, done.err);
  assert_eq!(done.out, format!("{result}\n"));
  assert!(done.err.contains(&running) && done.err.contains("remote says hi\n"), "{}", done.err);
  assert_eq!(git(&bench, &["rev-parse", "HEAD"]), git(&repo, &["rev-parse", "HEAD"]));
  assert_eq!(std::fs::read_to_string(bench.join("open.txt")).unwrap(), "not yet committed");
  assert!(!bench.join("ignored.txt").exists());
  assert_eq!(remote.asked.lock().unwrap()[0]["command"], json!(["cargo", "test"]));
}

// Whether the error stream says the run still runs on `machine`, at any of the seconds a test's run lasts.
fn says_still_running(err: &str, machine: &str) -> bool {
  (0..30).any(|seconds| {
    err.contains(&fill_marks(
      &LANES.placed.still_running,
      &[("machine", machine), ("seconds", &seconds.to_string())],
    ))
  })
}

// "every minute in which no byte of its output came over the connection, the caller says on standard error that
// it still runs there": through the endpoint, a run whose stream stays quiet for several of the test's short
// minutes is said to run on there, the line naming the endpoint, and ends with its stream's code.
#[test]
fn a_run_placed_through_the_endpoint_whose_stream_stays_quiet_says_it_still_runs() {
  let dir = tempfile::tempdir().unwrap();
  let repo = dir.path().join("repo");
  let ssh = dir.path().join("ssh");
  let result = "testbed: ok in 1s (waited 0s), cargo: 1 passed, 0 failed, 0 skipped; log: /x.log";
  let remote = endpoint_quiet(
    bench_answer(dir.path(), &ssh),
    vec![("result", json!({ "line": result, "code": 0 }).to_string())],
    Duration::from_millis(400),
  );

  install(SSH, &ssh);
  checkout(&repo, "testbed = 0\n");

  let place = format!("{}/place", remote.base);
  let done = ran(
    testbed(&repo, dir.path(), &["run", "--", "cargo", "test"])
      .env("BMO_TESTBED_PLACEMENT", &place)
      .env(STANDING_VARIABLE, "50"),
  );

  assert_eq!(done.code, 0, "{}\n{}", done.out, done.err);
  assert_eq!(done.out, format!("{result}\n"));
  assert!(says_still_running(&done.err, &place), "{}", done.err);
}

// "a stream that ends with no result, or a bench that cannot be made ready, ends the run as not run"
#[test]
fn a_run_dropped_before_its_result_ends_as_not_run() {
  let dir = tempfile::tempdir().unwrap();
  let repo = dir.path().join("repo");
  let ssh = dir.path().join("ssh");
  let refusing = dir.path().join("refusing-ssh");
  let silent = endpoint(bench_answer(dir.path(), &ssh), vec![("output", "starting".to_owned())]);
  let unready = endpoint(bench_answer(dir.path(), &refusing), Vec::new());
  let dropped = |said: &str| fill_marks(&LANES.run.dropped, &[("machine", "machine-2"), ("said", said)]);

  install(SSH, &ssh);
  install("#!/bin/sh\necho 'no route to stand' >&2\nexit 255\n", &refusing);
  checkout(&repo, "testbed = 0\n");

  let ended = placed(&repo, dir.path(), &silent.base, &["run", "--", "cargo", "test"]);

  assert_eq!(ended.code, i32::from(NOT_RUN_EXIT_CODE), "{}", ended.err);
  assert_eq!(ended.out, format!("{}\n", dropped(&LANES.placed.no_result)));

  let never = placed(&repo, dir.path(), &unready.base, &["run", "--", "cargo", "test"]);
  let failed = fill_marks(&LANES.placed.failed, &[("said", "no route to stand")]);

  assert_eq!(never.code, i32::from(NOT_RUN_EXIT_CODE), "{}", never.err);
  assert_eq!(never.out, format!("{}\n", dropped(&failed)));
}

// The settings naming this machine as the placed one, reached through the stand-in `ssh`, with the bench
// root and the testbed binary under the directory given.
fn settle(dir: &Path, ssh: &Path, destination: &str) {
  let env = dir.join("env");

  std::fs::create_dir_all(&env).unwrap();
  std::fs::write(
    env.join("placement.toml"),
    format!(
      "machine = \"machine-2\"\ndestination = \"{destination}\"\nbench = \"{}\"\nssh = [\"{}\"]\ntestbed = \"{}\"\n",
      dir.join("bench").display(),
      ssh.display(),
      env!("CARGO_BIN_EXE_bmo-testbed"),
    ),
  )
  .unwrap();
}

// A run of the binary in a checkout under the directory as the account's home, placed by the settings.
fn on_machine(repo: &Path, dir: &Path, args: &[&str]) -> Ran {
  ran(testbed(repo, dir, args).env("HOME", dir).env_remove("BMO_TESTBED_PLACEMENT"))
}

// "every minute in which no byte of its output came over the connection, the caller says on standard error that
// it still runs there": a run on the machine the settings name whose command there writes to its log and says
// nothing for several of the test's short minutes is said to run on there.
#[test]
fn a_run_on_the_placed_machine_whose_output_does_not_come_back_says_it_still_runs() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let repo = dir.join("repo");
  let ssh = dir.join("ssh");

  install(SSH, &ssh);
  settle(&dir, &ssh, "stand");
  checkout(&repo, "testbed = 0\n");

  let done = ran(
    testbed(&repo, &dir, &["run", "--adapter", "plain", "--", "sh", "-c", "sleep 0.4"])
      .env("HOME", &dir)
      .env_remove("BMO_TESTBED_PLACEMENT")
      .env(STANDING_VARIABLE, "50"),
  );

  assert_eq!(done.code, 0, "{}\n{}", done.out, done.err);
  assert!(says_still_running(&done.err, "machine-2"), "{}", done.err);
}

// "the placed machine the settings name runs the command in a bench there, set up first, and its code and
// output come back"
#[test]
fn a_machine_the_settings_name_runs_the_command_in_a_bench_there() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let repo = dir.join("repo");
  let ssh = dir.join("ssh");

  install(SSH, &ssh);
  settle(&dir, &ssh, "stand");
  checkout(
    &repo,
    "testbed = 0\n\n[test]\nsetup = [\"cp \\\"$BMO_TESTBED_CHECKOUTS\\\" set-up.txt\"]\n",
  );
  std::fs::write(repo.join(".gitignore"), "ignored.txt\nset-up.txt\n").unwrap();
  std::fs::write(repo.join("ignored.txt"), "left out").unwrap();
  std::fs::write(repo.join("open.txt"), "not yet committed").unwrap();

  let done = on_machine(&repo, &dir, &["run", "--adapter", "plain", "--", "sh", "-c", "echo far; exit 3"]);
  let bench = dir.join("bench/repo");
  let running =
    fill_marks(&LANES.placed.running, &[("machine", "machine-2"), ("bench", &bench.to_string_lossy())]);

  assert_eq!(done.code, 3, "{}\n{}", done.out, done.err);
  assert!(done.err.contains(&running), "{}", done.err);
  assert!(done.out.contains("testbed: FAILED"), "{}", done.out);
  assert_eq!(git(&bench, &["rev-parse", "HEAD"]), git(&repo, &["rev-parse", "HEAD"]));
  assert_eq!(std::fs::read_to_string(bench.join("open.txt")).unwrap(), "not yet committed");
  // The declared setup ran in the bench, handed the run's own registry, which names the bench and is taken away
  // as the run ends.
  let registry =
    std::fs::read_to_string(bench.join("set-up.txt")).expect("the declared setup ran in the bench");

  assert!(registry.contains(&format!("\"{}\"", bench.display())), "{registry}");
  assert!(
    std::fs::read_dir(dir.join("testbed")).unwrap().all(|entry| !entry
      .unwrap()
      .file_name()
      .to_string_lossy()
      .starts_with("placed-checkouts-"))
  );
  assert!(!bench.join("ignored.txt").exists());
}

// "a run started in a folder below the checkout's top runs in the same folder of the bench", its setup at the
// bench's top; "a run started in a folder its checkout ignores is never placed, said on a line naming the folder"
#[test]
fn a_run_placed_from_a_folder_runs_in_that_folder_there_and_one_from_an_ignored_folder_stays_here() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let repo = dir.join("repo");
  let ssh = dir.join("ssh");

  install(SSH, &ssh);
  settle(&dir, &ssh, "stand");
  checkout(&repo, "testbed = 0\n\n[test]\nsetup = [\"pwd > set-up.txt\"]\n");
  // `build/**` ignores what the folder holds and not the folder itself, which git still names as left out.
  std::fs::write(repo.join(".gitignore"), "build/**\nset-up.txt\nwhere.txt\n").unwrap();
  std::fs::create_dir_all(repo.join("studio/client")).unwrap();
  std::fs::create_dir_all(repo.join("build/out")).unwrap();
  std::fs::write(repo.join("build/out/made.o"), "built").unwrap();

  let folder = repo.join("studio/client");
  let done = on_machine(&folder, &dir, &["run", "--adapter", "plain", "--", "sh", "-c", "pwd > where.txt"]);
  let bench = dir.join("bench/repo");

  assert_eq!(done.code, 0, "{}\n{}", done.out, done.err);
  assert_eq!(
    std::fs::read_to_string(bench.join("studio/client/where.txt")).expect("the command ran in the folder"),
    format!("{}\n", bench.join("studio/client").display())
  );
  assert_eq!(
    std::fs::read_to_string(bench.join("set-up.txt")).unwrap(),
    format!("{}\n", bench.display())
  );

  let ignored = repo.join("build");
  let here =
    on_machine(&ignored, &dir, &["run", "--adapter", "plain", "--", "sh", "-c", "pwd > where.txt"]);
  let kept = fill_marks(&LANES.placed.ignored_dir, &[("dir", &ignored.to_string_lossy())]);

  assert_eq!(here.code, 0, "{}\n{}", here.out, here.err);
  assert!(here.err.contains(&kept), "{}", here.err);
  assert!(ignored.join("where.txt").exists());
  assert!(!bench.join("build").exists());
}

// "The copy carries what git itself sees in the tree": an untracked file a `!` line takes back reaches the
// bench, a file deleted here is deleted there, an ignored file the bench already holds stays, and a file git
// ignores by a name holding a wildcard is left out alone.
#[test]
fn a_placed_run_s_copy_carries_what_git_sees_a_file_a_negation_takes_back_among_them() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let repo = dir.join("repo");
  let bench = dir.join("bench/repo");
  let ssh = dir.join("ssh");

  install(SSH, &ssh);
  settle(&dir, &ssh, "stand");
  checkout(&repo, "testbed = 0\n");
  std::fs::write(repo.join(".gitignore"), "out/*\n!out/fresh/\nlogs/\nc\\*.log\n").unwrap();
  git(&repo, &["add", "."]);
  git(&repo, &["commit", "-q", "-m", "ignores"]);
  std::fs::create_dir_all(repo.join("out/fresh")).unwrap();
  std::fs::create_dir_all(repo.join("out/stale")).unwrap();
  std::fs::write(repo.join("out/fresh/taken-back.txt"), "taken back").unwrap();
  std::fs::write(repo.join("out/stale/left.txt"), "left out").unwrap();
  std::fs::write(repo.join("c*.log"), "left out").unwrap();
  std::fs::write(repo.join("cx.log"), "copied").unwrap();
  // What an earlier run left in the bench: an ignored file of its own, and a file no longer here.
  std::fs::create_dir_all(bench.join("logs")).unwrap();
  std::fs::write(bench.join("logs/far.log"), "the bench's own").unwrap();
  std::fs::write(bench.join("gone.txt"), "deleted here").unwrap();

  let done = on_machine(&repo, &dir, &["run", "--adapter", "plain", "--", "true"]);

  assert_eq!(done.code, 0, "{}\n{}", done.out, done.err);
  assert_eq!(std::fs::read_to_string(bench.join("out/fresh/taken-back.txt")).unwrap(), "taken back");
  assert_eq!(std::fs::read_to_string(bench.join("cx.log")).unwrap(), "copied");
  assert_eq!(std::fs::read_to_string(bench.join("logs/far.log")).unwrap(), "the bench's own");
  assert!(!bench.join("out/stale").exists());
  assert!(!bench.join("c*.log").exists());
  assert!(!bench.join("gone.txt").exists());
}

// A build in the bench that reads its source by modification time, as Cargo does: it compiles `lib.rs` into
// `built` where nothing is built yet or the source is newer than what it built.
const BUILD_BY_TIME: &str =
  r#"if [ ! -e built ] || [ -n "$(find lib.rs -newer built)" ]; then cp lib.rs built; fi"#;

// "The copy goes by content, not by time": a source edited here under a time older than the bench's last
// build is compiled by the next build there, open or committed, and a file whose content the bench already
// holds keeps its time there.
#[test]
fn a_file_copied_over_a_built_bench_under_an_older_time_is_compiled_by_the_next_build() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let repo = dir.join("repo");
  let bench = dir.join("bench/repo");
  let ssh = dir.join("ssh");
  let build = ["run", "--adapter", "plain", "--", "sh", "-c", BUILD_BY_TIME];
  let modified = |file: &Path| std::fs::metadata(file).unwrap().modified().unwrap();
  let edit_before_the_build = |text: &str| {
    std::fs::write(repo.join("lib.rs"), text).unwrap();
    std::fs::File::options()
      .write(true)
      .open(repo.join("lib.rs"))
      .unwrap()
      .set_modified(std::time::SystemTime::now() - Duration::from_secs(3600))
      .unwrap();
  };

  install(SSH, &ssh);
  settle(&dir, &ssh, "stand");
  checkout(&repo, "testbed = 0\n");
  std::fs::write(repo.join(".gitignore"), "built\n").unwrap();
  std::fs::write(repo.join("lib.rs"), "one").unwrap();
  std::fs::write(repo.join("other.rs"), "unchanged").unwrap();
  git(&repo, &["add", "."]);
  git(&repo, &["commit", "-q", "-m", "sources"]);

  let first = on_machine(&repo, &dir, &build);

  assert_eq!(first.code, 0, "{}\n{}", first.out, first.err);
  assert_eq!(std::fs::read_to_string(bench.join("built")).unwrap(), "one");
  let unchanged = modified(&bench.join("other.rs"));

  // An open edit whose time here is older than the build there.
  edit_before_the_build("two");
  let open = on_machine(&repo, &dir, &build);

  assert_eq!(open.code, 0, "{}\n{}", open.out, open.err);
  assert_eq!(std::fs::read_to_string(bench.join("built")).unwrap(), "two");

  // The same, committed: the bench's checkout writes it, and the copy of the open files after it leaves it.
  edit_before_the_build("three");
  git(&repo, &["commit", "-q", "-a", "-m", "three"]);
  let committed = on_machine(&repo, &dir, &build);

  assert_eq!(committed.code, 0, "{}\n{}", committed.out, committed.err);
  assert_eq!(std::fs::read_to_string(bench.join("built")).unwrap(), "three");
  assert_eq!(modified(&bench.join("other.rs")), unchanged);
}

// "Where the bench holds no such folder, nothing runs and the verb exits 2 on a line naming it."
#[test]
fn the_placed_verb_refuses_a_folder_the_bench_does_not_hold() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let repo = dir.join("repo");

  checkout(&repo, "testbed = 0\n");

  let done = ran(&mut testbed(
    &repo,
    &dir,
    &["placed", "--dir", "gone", "--", "sh", "-c", "touch ran.txt"],
  ));
  let refused = fill_marks(&LANES.placed.no_dir, &[("bench", &repo.to_string_lossy()), ("dir", "gone")]);

  assert_eq!(done.code, 2, "{}", done.err);
  assert!(done.err.contains(&refused), "{}", done.err);
  assert!(!repo.join("ran.txt").exists());
}

// "The commit its `--since` names is resolved here and handed over as the commit itself, since a name such as
// `origin/main` names a commit only in this checkout"
#[test]
fn a_placed_change_check_is_handed_the_commit_its_since_names_rather_than_the_name() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let repo = dir.join("repo");
  let ssh = dir.join("ssh");
  let line = dir.join("placed-line");

  // The `placed` line is written down and ends there; every other line runs as `SSH` runs it.
  install(
    &SSH.replacen(
      "shift\nexec",
      &format!(
        "shift\ncase \"$*\" in *\" placed \"*) echo \"$*\" > {}; exit 0 ;; esac\nexec",
        line.display()
      ),
      1,
    ),
    &ssh,
  );
  settle(&dir, &ssh, "stand");
  checkout(&repo, "testbed = 0\n");
  git(&repo, &["branch", "base"]);

  let base = git(&repo, &["rev-parse", "base"]);
  let done = on_machine(&repo, &dir, &["check", "--since", "base"]);
  let handed = std::fs::read_to_string(&line).expect("the check was placed");

  assert_eq!(done.code, 0, "{}\n{}", done.out, done.err);
  assert!(handed.contains(&format!(" --since {base}")), "{handed}");
  assert!(!handed.contains(" --since base"), "{handed}");
}

// "a bench whose place stands as a plain folder, a tree an earlier run copied there, is made a repository in it"
#[test]
fn a_bench_whose_place_stands_as_a_copied_folder_is_made_a_repository_there() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let repo = dir.join("repo");
  let ssh = dir.join("ssh");
  let bench = dir.join("bench/repo");

  install(SSH, &ssh);
  settle(&dir, &ssh, "stand");
  checkout(&repo, "testbed = 0\n");
  std::fs::create_dir_all(&bench).unwrap();
  std::fs::write(bench.join("copied.txt"), "an earlier run's copy").unwrap();

  let done = on_machine(&repo, &dir, &["run", "--adapter", "plain", "--", "sh", "-c", "exit 0"]);

  assert_eq!(done.code, 0, "{}\n{}", done.out, done.err);
  assert_eq!(git(&bench, &["rev-parse", "HEAD"]), git(&repo, &["rev-parse", "HEAD"]));
}

// "the copy of the checkout holding the shared versions leaves their file out, and the caller writes it there
// as that checkout's main branch holds it, whole"
#[test]
fn the_shared_versions_are_laid_over_their_copy_as_their_main_branch_holds_them() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let repo = dir.join("repo");
  let shared = dir.join("shared");
  let ssh = dir.join("ssh");
  let copy = dir.join("bench/shared");
  let on_main = "[snapshot]\nrt = \"1.0.0\"\n";

  install(SSH, &ssh);
  settle(&dir, &ssh, "stand");
  checkout(&shared, "testbed = 0\nname = \"shared-base\"\n");
  std::fs::write(shared.join("shared-versions.toml"), on_main).unwrap();
  git(&shared, &["add", "."]);
  git(&shared, &["commit", "-q", "-m", "pin"]);
  git(&shared, &["checkout", "-q", "-b", "release-next"]);
  std::fs::write(shared.join("shared-versions.toml"), "[snapshot]\nrt = \"1.1.0\"\n").unwrap();
  git(&shared, &["commit", "-q", "-am", "pin the next"]);
  std::fs::write(shared.join("shared-versions.toml"), "[snapshot]\nrt = \"1.2.0\"\n").unwrap();
  std::fs::write(shared.join("other.txt"), "copied").unwrap();
  checkout(&repo, "testbed = 0\n\n[check.shared_versions]\nrepository = \"shared-base\"\n");
  std::fs::create_dir_all(dir.join("testbed")).unwrap();
  std::fs::write(
    dir.join("testbed/checkouts.toml"),
    format!("checkouts = [{:?}, {:?}]\n", repo.to_string_lossy(), shared.to_string_lossy()),
  )
  .unwrap();
  // What an earlier copy of the working tree left there.
  std::fs::create_dir_all(&copy).unwrap();
  std::fs::write(copy.join("shared-versions.toml"), "[snapshot]\nrt = \"1.2.0\"\n").unwrap();

  let done = on_machine(&repo, &dir, &["run", "--adapter", "plain", "--", "sh", "-c", "exit 0"]);

  assert_eq!(done.code, 0, "{}\n{}", done.out, done.err);
  assert_eq!(std::fs::read_to_string(copy.join("shared-versions.toml")).unwrap(), on_main);
  assert_eq!(std::fs::read_to_string(copy.join("other.txt")).unwrap(), "copied");
  assert!(
    std::fs::read_dir(&copy).unwrap().all(|entry| !entry
      .unwrap()
      .file_name()
      .to_string_lossy()
      .starts_with("shared-versions.toml."))
  );
}

// "each tree the declaration's `[test] sibling_trees` names by a path from the checkout ... a test opening a file
// in a neighbouring checkout reads what it reads here rather than whatever an earlier run left at that place. A
// sibling tree that does not stand here fails the step, naming it."
#[test]
fn a_sibling_tree_the_tests_read_is_laid_over_what_an_earlier_run_left_and_one_absent_is_not_run() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let repo = dir.join("repo");
  let sibling = dir.join("sibling");
  let ssh = dir.join("ssh");
  let copy = dir.join("bench/sibling");
  let reads = "cmp ../sibling/releases/pinned.tgz ../sibling/releases/expected.tgz";

  install(SSH, &ssh);
  settle(&dir, &ssh, "stand");
  checkout(&repo, "testbed = 0\n\n[test]\nsibling_trees = [\"../sibling\"]\n");

  let absent = on_machine(&repo, &dir, &["run", "--adapter", "plain", "--", "sh", "-c", reads]);

  assert_eq!(absent.code, i32::from(NOT_RUN_EXIT_CODE), "{}\n{}", absent.out, absent.err);
  assert!(absent.out.contains("../sibling"), "{}", absent.out);

  checkout(&sibling, "testbed = 0\nname = \"sibling-base\"\n");
  std::fs::create_dir_all(sibling.join("releases")).unwrap();
  std::fs::write(sibling.join("releases/pinned.tgz"), "the release").unwrap();
  std::fs::write(sibling.join("releases/expected.tgz"), "the release").unwrap();
  // What an earlier run left at the sibling's place there.
  std::fs::create_dir_all(copy.join("releases")).unwrap();
  std::fs::write(copy.join("releases/pinned.tgz"), "an earlier build").unwrap();

  let done = on_machine(&repo, &dir, &["run", "--adapter", "plain", "--", "sh", "-c", reads]);

  assert_eq!(done.code, 0, "{}\n{}", done.out, done.err);
  assert_eq!(std::fs::read_to_string(copy.join("releases/pinned.tgz")).unwrap(), "the release");
}

// "the run knows that repository once, by the copy of the checkout found through this machine's registry ...
// A worker's copy of that repository therefore reads the file as the primary checkout's main branch holds it":
// a placed check run from a worktree of the shared repository, its own file changed and not reading, passes.
#[test]
fn a_placed_check_from_a_copy_of_the_shared_repository_reads_the_primary_checkouts_file() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let shared = dir.join("shared");
  let copy = dir.join("shared-w2");
  let ssh = dir.join("ssh");

  install(SSH, &ssh);
  settle(&dir, &ssh, "stand");
  checkout(
    &shared,
    "testbed = 0\nname = \"shared-base\"\n\n[check]\nquick = [\"shared-versions\"]\n\n[check.shared_versions]\n\
     repository = \"shared-base\"\n",
  );
  std::fs::write(shared.join("shared-versions.toml"), "[snapshot]\nrt = \"1.0.0\"\n").unwrap();
  git(&shared, &["add", "."]);
  git(&shared, &["commit", "-q", "-m", "pin"]);
  git(&shared, &["worktree", "add", "-q", "--detach", &copy.to_string_lossy()]);
  std::fs::write(copy.join("shared-versions.toml"), "[snapshot\n").unwrap();
  std::fs::create_dir_all(dir.join("testbed")).unwrap();
  std::fs::write(
    dir.join("testbed/checkouts.toml"),
    format!("checkouts = [{:?}]\n", shared.to_string_lossy()),
  )
  .unwrap();

  let done = on_machine(
    &copy,
    &dir,
    &[
      "run",
      "--adapter",
      "plain",
      "--",
      env!("CARGO_BIN_EXE_bmo-testbed"),
      "check",
      "shared-versions",
    ],
  );

  assert_eq!(done.code, 0, "{}\n{}", done.out, done.err);
}

// "a bench whose checkout's remote the machine there cannot read is made empty and handed the commit"
#[test]
fn a_bench_whose_remote_the_machine_there_cannot_read_is_made_empty_and_handed_the_commit() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let repo = dir.join("repo");
  let ssh = dir.join("ssh");
  let bench = dir.join("bench/repo");

  install(SSH, &ssh);
  settle(&dir, &ssh, "stand");
  checkout(&repo, "testbed = 0\n");
  // A path the machine there does not hold, as a support checkout's main checkout is to it.
  git(&repo, &["remote", "add", "origin", &dir.join("elsewhere").to_string_lossy()]);

  let done = on_machine(&repo, &dir, &["run", "--adapter", "plain", "--", "sh", "-c", "exit 0"]);

  assert_eq!(done.code, 0, "{}\n{}", done.out, done.err);
  assert_eq!(git(&bench, &["rev-parse", "HEAD"]), git(&repo, &["rev-parse", "HEAD"]));
}

// "a support checkout's reachable remote is the one its main checkout names; a chain that ends on this machine,
// or comes back on itself, names none"
#[test]
fn the_reachable_remote_follows_a_support_checkouts_main_to_the_remote_it_names() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let main = dir.join("main");
  let support = dir.join("support");

  checkout(&main, "testbed = 0\n");
  git(&dir, &["clone", "-q", &main.to_string_lossy(), &support.to_string_lossy()]);
  assert_eq!(reachable_remote(&support), "");

  git(&main, &["remote", "add", "origin", "https://example.com/main.git"]);
  assert_eq!(reachable_remote(&support), "https://example.com/main.git");

  git(&main, &["remote", "set-url", "origin", &format!("file://{}", support.display())]);
  assert_eq!(reachable_remote(&support), "");
}

// "A file that does not read is no placement, said on a line"
#[test]
fn settings_that_do_not_read_leave_the_run_here() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let repo = dir.join("repo");

  checkout(&repo, "testbed = 0\n");
  std::fs::create_dir_all(dir.join("env")).unwrap();
  std::fs::write(dir.join("env/placement.toml"), "bench = 3\n").unwrap();

  let unsettled = on_machine(&repo, &dir, &["run", "--adapter", "plain", "--", "sh", "-c", "exit 5"]);
  let file = dir.join("env/placement.toml");

  assert_eq!(unsettled.code, 5, "{}\n{}", unsettled.out, unsettled.err);
  assert!(
    unsettled
      .err
      .contains(&format!("testbed: the placement settings {} do not read", file.display())),
    "{}",
    unsettled.err
  );
}

// "a caller at the front or standing next here stands the same on the placed machine"
#[test]
fn a_caller_keeps_its_place_in_the_line_on_the_placed_machine() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let repo = dir.join("repo");
  let ssh = dir.join("ssh");

  // A stand-in that, as `ssh` does, hands the far side none of the caller's variables but those its line sets.
  let unhanded = SSH.replace("exec sh -c", "unset BMO_TESTBED_FRONT BMO_TESTBED_NEXT; exec sh -c");

  install(&unhanded, &ssh);
  settle(&dir, &ssh, "stand");
  checkout(&repo, "testbed = 0\n");

  let probe =
    r#"test "$BMO_TESTBED_FRONT" = 1 && exit 10; test "$BMO_TESTBED_NEXT" = 1 && exit 11; exit 12"#;
  let args = ["run", "--adapter", "plain", "--", "sh", "-c", probe];
  let in_place = |variable: Option<&str>| {
    let mut command = testbed(&repo, &dir, &args);

    command.env("HOME", &dir).env_remove("BMO_TESTBED_PLACEMENT");

    if let Some(variable) = variable {
      command.env(variable, "1");
    }

    ran(&mut command)
  };

  let front = in_place(Some("BMO_TESTBED_FRONT"));
  let next = in_place(Some("BMO_TESTBED_NEXT"));
  let neither = in_place(None);

  assert_eq!(front.code, 10, "{}\n{}", front.out, front.err);
  assert_eq!(next.code, 11, "{}\n{}", next.out, next.err);
  assert_eq!(neither.code, 12, "{}\n{}", neither.out, neither.err);
}

// The release of a commit under the directory's testbed deploy tree, `deploy-releases/` beside the link `deploy`,
// its `bin/built-from` naming the commit and no testbed in it.
fn release_of(dir: &Path, commit: &str) -> PathBuf {
  let release = dir.join("deploy-releases").join(&commit[..12]);

  std::fs::create_dir_all(release.join("bin")).unwrap();
  std::fs::write(release.join("bin/built-from"), format!("{commit}\n")).unwrap();
  release
}

// The directory's testbed deploy tree moved onto the release of a commit, made where it does not stand.
fn go_live(dir: &Path, commit: &str) -> PathBuf {
  let release = release_of(dir, commit);
  let link = dir.join("deploy");
  let _ = std::fs::remove_file(&link);

  std::os::unix::fs::symlink(Path::new("deploy-releases").join(&commit[..12]), &link).unwrap();
  release
}

// The placed machine reached through the stand-in `ssh`, its testbed under the directory given with the commit
// `far` beside it where one is named, and the directory's testbed deploy tree live on the release of `live` where
// one is named, with no testbed in it; a run waits for it not at all.
fn released(dir: &Path, far: Option<&str>, live: Option<&str>) -> MachinePlacement {
  let ssh = dir.join("ssh");
  let bin = dir.join("far/bin");

  install(SSH, &ssh);
  std::fs::create_dir_all(&bin).unwrap();

  if let Some(far) = far {
    std::fs::write(bin.join("built-from"), format!("{far}\n")).unwrap();
  }

  if let Some(live) = live {
    go_live(dir, live);
  }

  MachinePlacement {
    settings: MachineSettings {
      destination: Some("stand".to_owned()),
      machine: Some("machine-2".to_owned()),
      bench: dir.join("bench").to_string_lossy().into_owned(),
      deploy_bench: dir.join("deploy-bench").to_string_lossy().into_owned(),
      ssh: vec![ssh.to_string_lossy().into_owned()],
      testbed: bin.join("bmo-testbed").to_string_lossy().into_owned(),
      cargo_target: dir.join("cargo-target").to_string_lossy().into_owned(),
      disk_free_gigabytes: None,
      disk_paths: Vec::new(),
      order: None,
      combined_order: None,
    },
    home: dir.to_path_buf(),
    deploy_tree: live.map(|_| dir.join("deploy")),
    data_root: dir.join("data"),
    hook: Hook {
      command: None,
      time: Duration::from_secs(60),
      repository: None,
      root: None,
    },
  }
}

const LIVE: &str = "1111111111111111111111111111111111111111";
const FAR: &str = "2222222222222222222222222222222222222222";
const NEWER: &str = "3333333333333333333333333333333333333333";
// A testbed that runs: it answers its help, as the copied one is asked there.
const RUNS: &str = "#!/bin/sh\nexit 0\n";

// "where they differ, the run is not run and the deployment's stage fails, on a line naming the machine and
// both commits"
#[test]
fn a_testbed_there_of_another_release_is_refused_naming_both_commits_and_no_bench_is_made() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let repo = dir.join("repo");

  checkout(&repo, "testbed = 0\n");

  let machine = released(&dir, Some(FAR), Some(LIVE));
  let commit = git(&repo, &["rev-parse", "HEAD"]);
  let refused = machine.ready(&repo, &commit, BenchFor::Deployment("repo")).map(|bench| bench.path);
  let stale = fill_marks(
    &LANES.placed.stale,
    &[("machine", "machine-2"), ("far", &FAR[..12]), ("live", &LIVE[..12])],
  );

  assert_eq!(refused, Err(stale));
  assert!(!dir.join("bench").exists());
  assert!(!dir.join("deploy-bench").exists());
}

// "Where they differ, or no such file reads there, it first brings the testbed there to the live release ... with
// the live release as the source"; "Where that does not take ... naming both commits and what the machine answered";
// the copied testbed "run there once, so a copy that does not run there is found at once"
#[test]
fn a_testbed_there_of_another_release_is_brought_to_the_live_one_and_a_copy_that_does_not_run_is_refused() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let machine = released(&dir, Some(FAR), Some(LIVE));
  let far_bin = dir.join("far/bin");
  let live_bin = release_of(&dir, LIVE).join("bin");
  let stale = fill_marks(
    &LANES.placed.stale,
    &[("machine", "machine-2"), ("far", &FAR[..12]), ("live", &LIVE[..12])],
  );
  let unbrought = |said: &str| fill_marks(&LANES.placed.unbrought, &[("stale", &stale), ("said", said)]);
  let uncarried = fill_marks(
    &LANES.placed.live_uncarried,
    &[
      ("release", &live_bin.parent().unwrap().to_string_lossy()),
      ("binary", "bin/bmo-testbed"),
    ],
  );

  assert_eq!(machine.brought_live(), Err(unbrought(&uncarried)));
  assert_eq!(std::fs::read_to_string(far_bin.join("built-from")).unwrap().trim(), FAR);

  install(
    "#!/bin/sh\necho 'cannot execute binary file' >&2\nexit 126\n",
    &live_bin.join("bmo-testbed"),
  );

  let unrun = fill_marks(&LANES.placed.testbed_unrun, &[("said", "cannot execute binary file")]);

  assert_eq!(machine.brought_live(), Err(unbrought(&unrun)));

  install(RUNS, &live_bin.join("bmo-testbed"));

  assert_eq!(machine.brought_live(), Ok(()));
  assert_eq!(std::fs::read_to_string(far_bin.join("bmo-testbed")).unwrap(), RUNS);
  assert_eq!(std::fs::read_to_string(far_bin.join("built-from")).unwrap().trim(), LIVE);
  assert_eq!(machine.live_there(), Ok(()));
}

// "The live release is the one the testbed's deploy tree names now, read at the moment of asking, never the release
// of the process that asks: a process started from an older release ... sees the newer release live and never
// brings that machine back to its own"
#[test]
fn a_process_that_outlives_its_release_brings_the_machine_to_the_newer_live_one_never_back() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  // The process asking started while LIVE was live, and goes on asking after NEWER went live.
  let machine = released(&dir, Some(FAR), Some(LIVE));
  let far_built_from = dir.join("far/bin/built-from");

  install(RUNS, &release_of(&dir, LIVE).join("bin/bmo-testbed"));
  assert_eq!(machine.brought_live(), Ok(()));
  assert_eq!(std::fs::read_to_string(&far_built_from).unwrap().trim(), LIVE);

  install(RUNS, &go_live(&dir, NEWER).join("bin/bmo-testbed"));
  assert_eq!(machine.brought_live(), Ok(()));
  assert_eq!(std::fs::read_to_string(&far_built_from).unwrap().trim(), NEWER);
  assert_eq!(machine.brought_live(), Ok(()), "a machine on the live release is left as it stands");
  assert_eq!(std::fs::read_to_string(&far_built_from).unwrap().trim(), NEWER);
}

// "where no such file reads there, the run is not run"; "a testbed running from no release compares nothing"
#[test]
fn the_live_release_passes_one_naming_none_is_refused_and_none_live_compares_nothing() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();

  assert_eq!(released(&dir, Some(LIVE), Some(LIVE)).live_there(), Ok(()));

  std::fs::remove_file(dir.join("far/bin/built-from")).unwrap();

  let unreleased = released(&dir, None, Some(LIVE)).live_there().unwrap_err();
  let file = dir.join("far/bin/built-from");
  let opening =
    fill_marks(&LANES.placed.unreleased, &[("machine", "machine-2"), ("file", &file.to_string_lossy())]);

  assert!(unreleased.starts_with(opening.split("{said}").next().unwrap()), "{unreleased}");
  assert!(unreleased.contains(&LIVE[..12]), "{unreleased}");
  assert_eq!(released(&dir, None, None).live_there(), Ok(()));
}

// The deployment's gate on the placed machine, bringing the testbed there to a release where `brings` says so.
fn bringing(machine: MachinePlacement, brings: bool) -> PlacedGate<'static> {
  PlacedGate {
    machine,
    repository: "repo".to_owned(),
    here: &Passed,
    checked: true,
    brings_testbed: brings,
  }
}

// "copies that binary onto `testbed` there and the release's `bin/built-from` beside it, then reads that
// `built-from` back and checks it names the commit put live"; "Nothing is copied where ... the caller sets
// `BMO_TESTBED_PLACEMENT` empty"; "a machine that cannot be reached ... is said on a line naming the machine and
// what it answered"
#[test]
fn the_testbed_there_is_brought_to_a_release_carrying_one_and_read_back_at_its_commit() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let release = dir.join("release");
  let machine = || released(&dir, None, None);
  let far_bin = dir.join("far/bin");

  // The machine there holds no testbed yet, so its `bin/` is made too.
  let unmade = machine();

  std::fs::remove_dir_all(&far_bin).unwrap();
  std::fs::create_dir_all(release.join("bin")).unwrap();
  install("#!/bin/sh\necho released\n", &release.join("bin/bmo-testbed"));
  std::fs::write(release.join("bin/built-from"), format!("{LIVE}\n")).unwrap();

  let far = |name: &str| far_bin.join(name).to_string_lossy().into_owned();

  assert_eq!(
    bringing(unmade, true).bring_testbed(&release, LIVE),
    Some(("machine-2".to_owned(), Ok(vec![far("bmo-testbed"), far("built-from")])))
  );

  for name in ["bmo-testbed", "built-from"] {
    assert_eq!(
      std::fs::read(far_bin.join(name)).unwrap(),
      std::fs::read(release.join("bin").join(name)).unwrap()
    );
  }

  let mode = std::os::unix::fs::PermissionsExt::mode(
    &std::fs::metadata(far_bin.join("bmo-testbed")).unwrap().permissions(),
  );

  assert_eq!(mode & 0o777, 0o755);
  assert_eq!(
    bringing(machine(), true).bring_testbed(&release, FAR),
    Some((
      "machine-2".to_owned(),
      Err(fill_marks(&LANES.placed.testbed_other, &[("far", &LIVE[..12]), ("live", &FAR[..12])]))
    ))
  );
  assert_eq!(bringing(machine(), false).bring_testbed(&release, LIVE), None);
  assert_eq!(bringing(machine(), true).bring_testbed(&dir.join("no-release"), LIVE), None);

  let refusing = dir.join("refusing-ssh");
  let mut unreached = machine();

  install("#!/bin/sh\necho 'no route to stand' >&2\nexit 255\n", &refusing);
  unreached.settings.ssh = vec![refusing.to_string_lossy().into_owned()];

  assert_eq!(
    bringing(unreached, true).bring_testbed(&release, LIVE),
    Some((
      "machine-2".to_owned(),
      Err(fill_marks(&LANES.placed.testbed_unreached, &[("said", "no route to stand")]))
    ))
  );

  let env = |placement: Option<&str>| -> Env {
    placement
      .map(|value| ("BMO_TESTBED_PLACEMENT".to_owned(), value.to_owned()))
      .into_iter()
      .collect()
  };

  assert!(!brings_testbed(&env(Some(""))));
  assert!(brings_testbed(&env(None)));
  assert!(brings_testbed(&env(Some("http://placement.invalid/place"))));
}

// A checkout holding one package, its manifest naming the dependency given by a path, committed.
fn package_checkout(dir: &Path, name: &str, dependency: Option<&str>) {
  checkout(dir, &format!("testbed = 0\nname = \"{name}\"\n"));
  package_on(dir, name, dependency);
}

// One package committed onto a repository that already stands, its manifest naming the dependency given
// by a path.
fn package_on(dir: &Path, name: &str, dependency: Option<&str>) {
  std::fs::create_dir_all(dir.join("src")).unwrap();

  let depends = dependency
    .map_or_else(String::new, |path| format!("\n[dependencies]\nbase = {{ path = \"{path}\" }}\n"));

  std::fs::write(
    dir.join("Cargo.toml"),
    format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n{depends}"),
  )
  .unwrap();
  std::fs::write(dir.join("src/lib.rs"), "").unwrap();
  git(dir, &["add", "."]);
  git(dir, &["commit", "-q", "-m", "package"]);
}

// "A deployment's bench stands apart from every run's": a run copying the deployed repository's working
// tree beside its own checkout writes nothing into the deployment's tree, and the deployment's bench made
// ready again leaves the run's copy as it stood.
#[test]
fn a_deployments_bench_stands_apart_from_a_run_copying_the_same_tree() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let base = dir.join("base");
  let consumer = dir.join("consumer");

  package_checkout(&base, "base", None);
  package_checkout(&consumer, "consumer", Some("../base"));

  let machine = released(&dir, None, None);
  let deployed = git(&base, &["rev-parse", "HEAD"]);
  let bench = machine.ready(&base, &deployed, BenchFor::Deployment("base")).unwrap();
  let deployed_tree = dir.join("deploy-bench/base/base");

  assert_eq!(Path::new(&bench.path), deployed_tree);
  assert_eq!(git(&deployed_tree, &["rev-parse", "HEAD"]), deployed);

  // The deployed repository's working tree moves on, and a run of the checkout naming it copies it as it
  // stands.
  std::fs::write(base.join("src/lib.rs"), "pub fn moved() {}\n").unwrap();

  let run = git(&consumer, &["rev-parse", "HEAD"]);
  let ran = machine.ready(&consumer, &run, BenchFor::Run).unwrap();
  let copied = dir.join("bench/base");

  assert_eq!(Path::new(&ran.path), dir.join("bench/consumer"));
  assert!(ran.checkouts.contains(&copied.to_string_lossy().into_owned()), "{:?}", ran.checkouts);
  assert_eq!(std::fs::read_to_string(copied.join("src/lib.rs")).unwrap(), "pub fn moved() {}\n");
  assert_eq!(std::fs::read_to_string(deployed_tree.join("src/lib.rs")).unwrap(), "");
  assert_eq!(git(&deployed_tree, &["status", "--porcelain"]), "");

  machine.ready(&base, &deployed, BenchFor::Deployment("base")).unwrap();

  assert_eq!(std::fs::read_to_string(copied.join("src/lib.rs")).unwrap(), "pub fn moved() {}\n");
}

// "A tree that no git repository holds has no ignore rules to read, and the run ends as not run on a line
// naming it rather than copying it whole."
#[test]
fn a_run_s_bench_refuses_an_outside_tree_no_repository_holds_naming_it() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let loose = dir.join("loose");
  let consumer = dir.join("consumer");

  std::fs::create_dir_all(loose.join("src")).unwrap();
  std::fs::write(
    loose.join("Cargo.toml"),
    "[package]\nname = \"loose\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
  )
  .unwrap();
  std::fs::write(loose.join("src/lib.rs"), "").unwrap();
  package_checkout(&consumer, "consumer", Some("../loose"));

  let machine = released(&dir, None, None);
  let run = git(&consumer, &["rev-parse", "HEAD"]);
  let Err(said) = machine.ready(&consumer, &run, BenchFor::Run) else {
    panic!("a tree no repository holds was copied");
  };
  let named = fill_marks(&LANES.placed.tree_uncopied, &[("tree", &loose.to_string_lossy()), ("said", "")]);

  assert!(said.starts_with(named.trim_end()), "{said}");
  assert!(!dir.join("bench/loose/src").exists());
}

// "For a deployment it lays no working tree: the repository holding each tree is ... pushed the head of the
// main branch its declaration names ... checked out detached and its untracked files taken away": a base
// library's checkout mid-work, on a job's branch with an edit half written over it, reaches a deployment's
// bench at its main head alone, while a run's bench copies it as it stands.
#[test]
fn a_deployments_bench_lays_each_dependency_at_its_main_head_never_its_working_tree() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let base = dir.join("base");
  let consumer = dir.join("consumer");

  package_checkout(&base, "base", None);
  std::fs::write(base.join("src/lib.rs"), "pub fn committed() {}\n").unwrap();
  git(&base, &["commit", "-q", "-am", "on main"]);
  package_checkout(&consumer, "consumer", Some("../base"));

  let main = git(&base, &["rev-parse", "main"]);

  git(&base, &["checkout", "-q", "-b", "job"]);
  std::fs::write(base.join("src/lib.rs"), "pub fn on_the_branch() {}\n").unwrap();
  git(&base, &["commit", "-q", "-am", "on the job's branch"]);
  std::fs::write(base.join("src/lib.rs"), "pub fn half_written(\n").unwrap();
  std::fs::write(base.join("untracked.rs"), "left by the job\n").unwrap();

  let machine = released(&dir, None, None);
  let deployed = git(&consumer, &["rev-parse", "HEAD"]);
  let bench = machine.ready(&consumer, &deployed, BenchFor::Deployment("consumer")).unwrap();
  let laid = dir.join("deploy-bench/consumer/base");

  assert!(bench.checkouts.contains(&laid.to_string_lossy().into_owned()), "{:?}", bench.checkouts);
  assert_eq!(git(&laid, &["rev-parse", "HEAD"]), main);
  assert_eq!(std::fs::read_to_string(laid.join("src/lib.rs")).unwrap(), "pub fn committed() {}\n");
  assert!(!laid.join("untracked.rs").exists());
  assert_eq!(git(&laid, &["status", "--porcelain"]), "");

  // A file an earlier laying or run left there goes with the next deployment's bench.
  std::fs::write(laid.join("stray.rs"), "an earlier copy").unwrap();
  machine.ready(&consumer, &deployed, BenchFor::Deployment("consumer")).unwrap();

  assert!(!laid.join("stray.rs").exists());

  let ran = machine.ready(&consumer, &deployed, BenchFor::Run).unwrap();
  let copied = dir.join("bench/base");

  assert!(ran.checkouts.contains(&copied.to_string_lossy().into_owned()), "{:?}", ran.checkouts);
  assert_eq!(std::fs::read_to_string(copied.join("src/lib.rs")).unwrap(), "pub fn half_written(\n");
}

// "The list of trees is read off Cargo's word from the workspace at the commit deployed, never from a checkout's
// working tree": a checkout left behind the deployed commit, which adds a path dependency, still has that
// dependency laid beside the deployment's bench.
#[test]
fn a_deployments_bench_lays_the_dependency_its_commit_adds_where_the_checkout_stands_behind() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let base = dir.join("base");
  let consumer = dir.join("consumer");

  package_checkout(&base, "base", None);
  package_checkout(&consumer, "consumer", None);
  package_on(&consumer, "consumer", Some("../base"));

  let deployed = git(&consumer, &["rev-parse", "HEAD"]);

  git(&consumer, &["checkout", "-q", "--detach", "HEAD~1"]);

  let machine = released(&dir, None, None);
  let bench = machine.ready(&consumer, &deployed, BenchFor::Deployment("consumer")).unwrap();
  let laid = dir.join("deploy-bench/consumer/base");

  assert!(bench.checkouts.contains(&laid.to_string_lossy().into_owned()), "{:?}", bench.checkouts);
  assert_eq!(git(&laid, &["rev-parse", "HEAD"]), git(&base, &["rev-parse", "main"]));
}

// "Each repository's deployments keep one bench there, at one path, whichever of the repository's checkouts a
// deployment is taken from"; "A bench another checkout of the repository left there goes ... The trees laid
// beside the bench are other repositories', whose roots differ, and stay."
#[test]
fn a_worktree_deploys_from_its_repositorys_one_bench_and_the_bench_another_checkout_left_goes() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let base = dir.join("base");
  let consumer = dir.join("consumer");
  let worktree = dir.join("consumer-w2");

  package_checkout(&base, "base", None);
  package_checkout(&consumer, "consumer", Some("../base"));
  git(&consumer, &["worktree", "add", "-q", "--detach", &worktree.to_string_lossy()]);

  let machine = released(&dir, None, None);
  let deployed = git(&worktree, &["rev-parse", "HEAD"]);
  let left = dir.join("deploy-bench/consumer/consumer-w3");

  git(&dir, &["clone", "-q", &consumer.to_string_lossy(), &left.to_string_lossy()]);

  let bench = machine.ready(&worktree, &deployed, BenchFor::Deployment("consumer")).unwrap();
  let one = dir.join("deploy-bench/consumer/consumer");
  let laid = dir.join("deploy-bench/consumer/base");

  assert_eq!(Path::new(&bench.path), one);
  assert_eq!(git(&one, &["rev-parse", "HEAD"]), deployed);
  assert_eq!(bench.heads, [git(&base, &["rev-parse", "main"])]);
  assert!(!left.exists());
  assert_eq!(git(&laid, &["rev-parse", "HEAD"]), git(&base, &["rev-parse", "main"]));
  assert!(!dir.join("deploy-bench/consumer/consumer-w2").exists());
}

// "The sweep never takes a tree laid for this deployment ... it is the laid list that keeps them apart": two
// repositories split from one history share their root commits, so a deployment of the consumer keeps the
// sibling laid as its dependency beside the one bench while the clone an earlier checkout left goes.
#[test]
fn a_deployments_bench_keeps_a_laid_sibling_sharing_its_root_commits() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let root = dir.join("root");
  let base = dir.join("base");
  let consumer = dir.join("consumer");

  // One history, split into two repositories that each hold one package: their root commits are the same.
  checkout(&root, "testbed = 0\n");
  git(&dir, &["clone", "-q", &root.to_string_lossy(), &base.to_string_lossy()]);
  git(&dir, &["clone", "-q", &root.to_string_lossy(), &consumer.to_string_lossy()]);
  package_on(&base, "base", None);
  package_on(&consumer, "consumer", Some("../base"));

  let machine = released(&dir, None, None);
  let deployed = git(&consumer, &["rev-parse", "HEAD"]);
  let left = dir.join("deploy-bench/consumer/consumer-w3");

  git(&dir, &["clone", "-q", &consumer.to_string_lossy(), &left.to_string_lossy()]);

  let bench = machine.ready(&consumer, &deployed, BenchFor::Deployment("consumer")).unwrap();
  let one = dir.join("deploy-bench/consumer/consumer");
  let laid = dir.join("deploy-bench/consumer/base");

  assert_eq!(Path::new(&bench.path), one);
  assert_eq!(git(&one, &["rev-parse", "HEAD"]), deployed);
  assert!(bench.checkouts.contains(&laid.to_string_lossy().into_owned()), "{:?}", bench.checkouts);
  assert_eq!(git(&laid, &["rev-parse", "HEAD"]), git(&base, &["rev-parse", "main"]));
  assert!(!left.exists());
}

// "where one does not once the bench is made ready, the bench is at fault and not the commit": a
// stand-in node that answers the laid tree's checkout as done without running it leaves the bench without
// that working tree, and the making of the bench fails naming the tree and blaming the bench.
#[test]
fn a_deployment_blames_its_bench_when_a_laid_sibling_does_not_stand() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let base = dir.join("base");
  let consumer = dir.join("consumer");
  let ssh = dir.join("ssh");
  let place = dir.join("deploy-bench/consumer/base");

  package_checkout(&base, "base", None);
  package_checkout(&consumer, "consumer", Some("../base"));

  let machine = released(&dir, None, None);

  // The stand-in `ssh` runs every line but the laid tree's checkout, which it answers as done; the tree's
  // repository stands from the push with no commit checked out.
  install(
    &SSH.replace(
      "exec sh -c",
      &format!(
        "case \"$*\" in *\"{}\"*\" && git checkout\"*) exit 0 ;; esac\nexec sh -c",
        place.display()
      ),
    ),
    &ssh,
  );

  let deployed = git(&consumer, &["rev-parse", "HEAD"]);
  let why = machine
    .ready(&consumer, &deployed, BenchFor::Deployment("consumer"))
    .map(|bench| bench.path)
    .unwrap_err();

  assert_eq!(
    why,
    fill_marks(
      &LANES.placed.trees_gone,
      &[("machine", "machine-2"), ("names", &place.to_string_lossy())]
    )
  );
}

// "a tree no repository holds ... fails the step"
#[test]
fn a_deployments_bench_refuses_a_dependency_no_repository_holds() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let base = dir.join("base");
  let consumer = dir.join("consumer");

  package_checkout(&base, "base", None);
  package_checkout(&consumer, "consumer", Some("../base"));
  std::fs::remove_dir_all(base.join(".git")).unwrap();

  let machine = released(&dir, None, None);
  let deployed = git(&consumer, &["rev-parse", "HEAD"]);
  let why = machine
    .ready(&consumer, &deployed, BenchFor::Deployment("consumer"))
    .map(|bench| bench.path)
    .unwrap_err();

  assert!(why.contains("no git repository holds it"), "{why}");
}

// The events the hook was handed, in order: their names, and the payload of the last.
fn sent(dir: &Path) -> (String, Value) {
  let payloads = std::fs::read_to_string(dir.join("payloads")).unwrap();
  let payload = serde_json::from_str(payloads.trim().lines().last().unwrap()).unwrap();

  (std::fs::read_to_string(dir.join("events")).unwrap(), payload)
}

// "Where that does not take while the machine answers ... a `run`, a suite, a change check or a Cargo verb passes
// on down the order and sends `placed-release-failed`"
#[test]
fn a_run_whose_machine_cannot_be_brought_to_the_live_release_passes_on_and_sends_its_event() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let repo = dir.join("repo");
  let marker = dir.join("ran-here");

  checkout(&repo, "testbed = 0\n");

  let command = vec!["touch".to_owned(), marker.to_string_lossy().into_owned()];
  let env = Env::new();
  let ask = PlaceAsk {
    command: &command,
    adapter: Some("plain"),
    lane: Lane::Wide,
    cwd: repo.clone(),
    env: &env,
  };
  let mut machine = released(&dir, Some(FAR), Some(LIVE));

  machine.hook = writing_hook(&dir);

  // The live release holds no testbed to copy, so the run passes on and nothing is laid there.
  assert_eq!(machine.place(&ask), None);
  assert!(!dir.join("bench").exists());
  assert!(!marker.exists());

  let (events, payload) = sent(&dir);

  assert_eq!(events, "placed-release-failed\n");
  assert_eq!(payload["detail"]["machine"], "machine-2");
  assert_eq!(payload["detail"]["live"], LIVE);
  assert_eq!(payload["detail"]["far"], FAR);
  assert_eq!(payload["detail"]["command"], json!(command));
}

// "Sent once for each live release ... A run that finds the testbed there at the live release clears this, so a
// failure after it is sent again"
#[test]
fn a_release_failure_is_told_once_for_each_live_release_and_again_once_a_run_found_it_brought() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let built_from = dir.join("far/bin/built-from");
  let command = vec!["true".to_owned()];
  let env = Env::new();
  let ask = PlaceAsk {
    command: &command,
    adapter: Some("plain"),
    lane: Lane::Wide,
    cwd: dir.clone(),
    env: &env,
  };
  let mut machine = released(&dir, Some(FAR), Some(LIVE));
  let events = || std::fs::read_to_string(dir.join("events")).unwrap();

  machine.hook = writing_hook(&dir);

  // The live release holds no testbed to copy, so the testbed there is never brought.
  assert!(!machine.ready_for(&ask));
  assert!(!machine.ready_for(&ask));
  assert_eq!(events(), "placed-release-failed\n", "one live release's failure is told once");

  go_live(&dir, NEWER);
  assert!(!machine.ready_for(&ask));
  assert_eq!(events(), "placed-release-failed\n".repeat(2), "a newer live release's is told again");

  std::fs::write(&built_from, format!("{NEWER}\n")).unwrap();
  assert!(machine.ready_for(&ask));
  std::fs::write(&built_from, format!("{FAR}\n")).unwrap();
  assert!(!machine.ready_for(&ask));
  assert_eq!(
    events(),
    "placed-release-failed\n".repeat(3),
    "a failure after the testbed was brought is told again"
  );
}

// "Where neither names a hook, the placed machine's events go to the owner's supervisor": through `notice`, an
// outage reaches the receiver's instructions as one notice however many runs give up on it, and its end as one more
#[test]
fn an_outage_and_its_end_reach_the_owner_s_supervisor_as_one_notice_each_through_notice() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let refusing = dir.join("refusing-ssh");
  let receiver = ReceiverStand::answering(201);
  let mut machine = released(&dir, None, None);
  let answering = machine.settings.ssh.clone();

  install("#!/bin/sh\necho 'no agent attached' >&2\nexit 255\n", &refusing);
  receiver.written_under(&machine.data_root, "stand-token");
  // The binary's own `notice`, on the test's data root and on no receiver the test's environment names.
  machine.hook.command = Some(format!(
    "env -u RECEIVER_ADDRESS -u RECEIVER_TOKEN BMO_DATA_DIR='{}' '{}' notice",
    machine.data_root.display(),
    env!("CARGO_BIN_EXE_bmo-testbed")
  ));
  machine.settings.ssh = vec![refusing.to_string_lossy().into_owned()];

  let command = vec!["true".to_owned()];
  let env = Env::new();
  let ask = PlaceAsk {
    command: &command,
    adapter: Some("plain"),
    lane: Lane::Wide,
    cwd: dir.clone(),
    env: &env,
  };
  let codes = || receiver.noticed().iter().map(|one| one.body["kind"].clone()).collect::<Vec<_>>();

  assert!(!machine.ready_for(&ask));
  assert!(!machine.ready_for(&ask));
  assert_eq!(codes(), [json!("testbed:placed-unreachable")], "one outage is one notice");
  assert_eq!(receiver.noticed()[0].body["said"]["fact"]["detail"]["machine"], "machine-2");

  machine.settings.ssh = answering;
  assert!(machine.ready_for(&ask));
  assert!(machine.ready_for(&ask));
  assert_eq!(
    codes(),
    [json!("testbed:placed-unreachable"), json!("testbed:placed-recovered")],
    "its end is one notice more"
  );
}

// "A machine that cannot be reached passes the run on ... The first run that passes on for it sends
// `placed-unreachable`", once for the outage however many runs pass on; the machine answering again ends the outage
// and sends `placed-recovered` once; the events go to the hook the machine's testbed environment file names, plain
// or in quotes
#[test]
fn an_outage_is_told_once_however_many_runs_pass_on_and_its_end_once_to_the_machine_s_hook() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let repo = dir.join("repo");
  let refusing = dir.join("refusing-ssh");
  let marker = dir.join("ran-here");
  let hook = writing_hook(&dir).command.unwrap();
  let env_file = dir.join("data").join(MACHINE_ENV_FILE);

  checkout(&repo, "testbed = 0\n");
  install("#!/bin/sh\necho 'no agent attached' >&2\nexit 255\n", &refusing);
  std::fs::create_dir_all(env_file.parent().unwrap()).unwrap();
  std::fs::write(&env_file, format!("OTHER=1\nBMO_TESTBED_EVENT_HOOK=\"{hook}\"\n")).unwrap();

  let mut machine = released(&dir, None, None);
  let answering = machine.settings.ssh.clone();

  machine.settings.ssh = vec![refusing.to_string_lossy().into_owned()];
  machine.settings.order = Some(vec!["machine-2".to_owned(), LANES.placed.here.clone()]);
  machine.hook = machine_hook(&machine.data_root, &Env::new());

  let command = vec!["touch".to_owned(), marker.to_string_lossy().into_owned()];
  let env = Env::new();
  let ask = PlaceAsk {
    command: &command,
    adapter: Some("plain"),
    lane: Lane::Wide,
    cwd: repo.clone(),
    env: &env,
  };

  // The order names this machine after the one away, so each run passes on to run here.
  assert_eq!(machine.place(&ask), None);
  assert_eq!(machine.place(&ask), None);
  assert!(!marker.exists(), "the placement runs nothing itself");

  let (events, payload) = sent(&dir);

  assert_eq!(events, "placed-unreachable\n", "one outage is told once");
  assert_eq!(payload["detail"]["machine"], "machine-2");
  assert_eq!(payload["detail"]["said"], "no agent attached");
  assert!(payload["detail"]["since"].is_string(), "{payload}");

  let since = payload["detail"]["since"].clone();

  machine.settings.ssh = answering;
  assert_eq!(machine.answers(), Ok(()));
  assert_eq!(machine.answers(), Ok(()));

  let (events, payload) = sent(&dir);

  assert_eq!(events, "placed-unreachable\nplaced-recovered\n", "its end is told once");
  assert_eq!(payload["detail"]["since"], since);

  machine.settings.ssh = vec![refusing.to_string_lossy().into_owned()];
  assert_eq!(machine.place(&ask), None);
  assert_eq!(
    sent(&dir).0,
    "placed-unreachable\nplaced-recovered\nplaced-unreachable\n",
    "a new outage is told again"
  );
}

// An outage that ends before any run passed on for it was never told, so its end is not told either.
#[test]
fn an_outage_no_run_passed_on_for_ends_untold() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let refusing = dir.join("refusing-ssh");
  let mut machine = released(&dir, None, None);
  let answering = machine.settings.ssh.clone();

  install("#!/bin/sh\necho 'no agent attached' >&2\nexit 255\n", &refusing);
  machine.hook = writing_hook(&dir);
  machine.settings.ssh = vec![refusing.to_string_lossy().into_owned()];
  assert!(machine.answers().is_err());

  machine.settings.ssh = answering;
  assert_eq!(machine.answers(), Ok(()));
  assert!(!dir.join("events").exists());
}

// "the environment's `BMO_TESTBED_EVENT_HOOK`, else the one the machine's testbed environment file names, else
// none"
#[test]
fn the_machine_s_hook_is_the_environment_s_else_its_testbed_environment_file_s() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let file = dir.join(MACHINE_ENV_FILE);
  let named = |env: &Env| machine_hook(&dir, env).command;
  let none = Env::new();

  assert_eq!(named(&none), None);

  std::fs::create_dir_all(file.parent().unwrap()).unwrap();
  std::fs::write(
    &file,
    "BMO_CARGO_TARGET_CEILING_GB=300\nBMO_TESTBED_EVENT_HOOK=/opt/tell --to supervisor\n",
  )
  .unwrap();
  assert_eq!(named(&none).as_deref(), Some("/opt/tell --to supervisor"));

  std::fs::write(&file, "BMO_TESTBED_EVENT_HOOK='/opt/tell --to supervisor'\n").unwrap();
  assert_eq!(named(&none).as_deref(), Some("/opt/tell --to supervisor"));

  let set = Env::from([("BMO_TESTBED_EVENT_HOOK".to_owned(), "/opt/other".to_owned())]);

  assert_eq!(named(&set).as_deref(), Some("/opt/other"));
}

// "Nothing waits for a machine to come back": a machine that does not answer is asked once, and the run passes on
#[test]
fn a_run_passes_on_at_once_from_a_machine_out_of_reach() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let ssh = dir.join("ssh");
  let asked = dir.join("asked");
  let mut machine = released(&dir, None, None);

  // The machine would answer from the fourth time it is asked on.
  install(
    &dropping_ssh(&dir, 0).replacen(
      "shift\n",
      &format!(
        "shift\nn=$(( $(cat {asked} 2>/dev/null || echo 0) + 1 ))\necho $n > {asked}\n\
         if [ $n -le 3 ]; then echo 'no agent attached' >&2; exit 255; fi\n",
        asked = asked.display()
      ),
      1,
    ),
    &ssh,
  );
  machine.settings.ssh = vec![ssh.to_string_lossy().into_owned()];
  machine.settings.testbed = env!("CARGO_BIN_EXE_bmo-testbed").to_owned();
  checkout(&dir.join("repo"), "testbed = 0\n");

  let command = vec!["sh".to_owned(), "-c".to_owned(), "exit 7".to_owned()];
  let env = Env::new();
  let ask = PlaceAsk {
    command: &command,
    adapter: Some("plain"),
    lane: Lane::Wide,
    cwd: dir.join("repo"),
    env: &env,
  };

  assert_eq!(machine.place(&ask), None);

  let times: usize = std::fs::read_to_string(&asked).unwrap().trim().parse().unwrap();

  assert_eq!(times, 1, "it was asked {times} times");
}

// "A connection that breaks once the run has begun passes the run on too ... the run starts again from its
// beginning on the next machine in the order that takes it, or here"
#[test]
fn a_run_whose_connection_breaks_under_it_passes_on() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let ssh = dir.join("ssh");
  let mut machine = released(&dir, None, None);

  // The run's own line there breaks; every other line answers.
  install(&dropping_ssh(&dir, 1), &ssh);
  machine.settings.ssh = vec![ssh.to_string_lossy().into_owned()];
  machine.settings.testbed = env!("CARGO_BIN_EXE_bmo-testbed").to_owned();
  checkout(&dir.join("repo"), "testbed = 0\n");

  let command = vec!["sh".to_owned(), "-c".to_owned(), "exit 7".to_owned()];
  let env = Env::new();
  let ask = PlaceAsk {
    command: &command,
    adapter: Some("plain"),
    lane: Lane::Wide,
    cwd: dir.join("repo"),
    env: &env,
  };

  assert_eq!(machine.place(&ask), None, "the broken run passes on");
  assert_eq!(machine.place(&ask), Some(7), "a run that ends there gives its own code");
}

// `status` reads the placed machine: "whether it answers, and where it does, the commit its testbed is built from and
// whether that is this machine's live testbed release"
#[test]
fn status_reads_the_machine_s_release_against_the_live_one_and_says_one_out_of_reach() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let state = |machine: &MachinePlacement| machine.state();
  let current = state(&released(&dir, Some(LIVE), Some(LIVE)));

  assert_eq!(
    current,
    MachineState {
      machine: "machine-2".to_owned(),
      reached: true,
      said: None,
      outage_since: None,
      release: Some(LIVE.to_owned()),
      live: Some(LIVE.to_owned()),
      current: true,
    }
  );
  assert_eq!(
    current.line(),
    fill_marks(
      &LANES.status.machine,
      &[
        ("machine", "machine-2"),
        ("state", &fill_marks(&LANES.status.machine_current, &[("release", &LIVE[..12])])),
      ]
    )
  );

  let behind = state(&released(&dir, Some(FAR), Some(LIVE)));

  assert!(!behind.current && behind.reached, "{behind:?}");
  assert!(
    behind.line().ends_with(&fill_marks(
      &LANES.status.machine_behind,
      &[("release", &FAR[..12]), ("live", &LIVE[..12])]
    )),
    "{}",
    behind.line()
  );

  let refusing = dir.join("refusing-ssh");
  let mut away = released(&dir, Some(LIVE), Some(LIVE));

  install("#!/bin/sh\necho 'no agent attached' >&2\nexit 255\n", &refusing);
  away.settings.ssh = vec![refusing.to_string_lossy().into_owned()];

  // A run found it away, so an outage stands.
  assert!(away.answers().is_err());

  let unreached = state(&away);
  let since = unreached.outage_since.clone().expect("the outage a run began");
  let outage = fill_marks(&LANES.status.machine_outage, &[("since", &since)]);

  assert!(!unreached.reached && !unreached.current, "{unreached:?}");
  assert_eq!(unreached.said.as_deref(), Some("no agent attached"));
  assert!(
    unreached.line().ends_with(&fill_marks(
      &LANES.status.machine_unreached,
      &[("said", "no agent attached"), ("outage", &outage)]
    )),
    "{}",
    unreached.line()
  );
}

// "a run whose environment sets a variable its checkout's declaration names in `[runner] local_variables` is
// never placed, said on a line naming the variable"
#[test]
fn a_run_setting_a_variable_its_declaration_names_as_this_machines_stays_here() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let repo = dir.join("repo");
  let ssh = dir.join("ssh");

  install(SSH, &ssh);
  settle(&dir, &ssh, "stand");
  checkout(&repo, "testbed = 0\n\n[runner]\nlocal_variables = [\"CONSOLE_URL\"]\n");

  let here = ran(
    testbed(&repo, &dir, &["run", "--adapter", "plain", "--", "sh", "-c", "exit 6"])
      .env("HOME", &dir)
      .env("CONSOLE_URL", "http://127.0.0.1:1"),
  );
  let kept = fill_marks(&LANES.placed.kept, &[("named", "CONSOLE_URL")]);

  assert_eq!(here.code, 6, "{}\n{}", here.out, here.err);
  assert!(here.err.contains(&kept), "{}", here.err);
  assert!(!dir.join("bench").exists());
}

// "a run started in a checkout holding a file its declaration names in `[runner] local_files` is never placed,
// said on a line naming the file"
#[test]
fn a_run_in_a_checkout_holding_a_file_its_declaration_names_as_this_machines_stays_here() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let repo = dir.join("repo");
  let ssh = dir.join("ssh");

  install(SSH, &ssh);
  settle(&dir, &ssh, "stand");
  checkout(&repo, "testbed = 0\n\n[runner]\nlocal_files = [\".env.local\"]\n");
  std::fs::write(repo.join(".env.local"), "SERVICE_URL=http://127.0.0.1:1\n").unwrap();

  let here =
    ran(testbed(&repo, &dir, &["run", "--adapter", "plain", "--", "sh", "-c", "exit 6"]).env("HOME", &dir));
  let kept = fill_marks(&LANES.placed.kept, &[("named", ".env.local")]);

  assert_eq!(here.code, 6, "{}\n{}", here.out, here.err);
  assert!(here.err.contains(&kept), "{}", here.err);
  assert!(!dir.join("bench").exists());
}

// "while a run placed on the machine lasts it stands in its lane here, and the line saying its turn there came is
// not passed on"
#[test]
fn a_run_placed_on_the_machine_stands_in_its_lane_here_while_it_lasts() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let repo = dir.join("repo");
  let ssh = dir.join("ssh");
  let seen = dir.join("seen.json");
  // The run there reads what this machine names it by while it runs: the far side shares this lock here.
  let copy = format!("cat {}/testbed.lock.placed/* > {}", dir.display(), seen.display());

  install(SSH, &ssh);
  settle(&dir, &ssh, "stand");
  checkout(&repo, "testbed = 0\n");

  let done = on_machine(&repo, &dir, &["run", "--adapter", "plain", "--", "sh", "-c", &copy]);
  let stood: serde_json::Value =
    serde_json::from_str(&std::fs::read_to_string(&seen).expect("the run there ran")).unwrap();

  assert_eq!(done.code, 0, "{}\n{}", done.out, done.err);
  assert_eq!(stood["machine"], "machine-2");
  assert_eq!(stood["checkout"], repo.to_string_lossy().as_ref());
  assert_eq!(stood["command"], format!("sh -c {copy}"));
  assert!(!done.err.contains(&LANES.placed.turn_came), "{}", done.err);

  let env =
    Env::from([("BMO_TESTBED_LOCK".to_owned(), dir.join("testbed.lock").to_string_lossy().into_owned())]);

  assert!(read_placed(&lock_path(&env, Lane::Wide)).is_empty(), "it goes as the run ends");
}

// "the placed run is running here once the far side says its turn there came"
#[test]
fn the_placed_run_is_running_once_the_far_side_says_its_turn_came() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let placement = released(&dir, None, None);
  let env =
    Env::from([("BMO_TESTBED_LOCK".to_owned(), dir.join("testbed.lock").to_string_lossy().into_owned())]);
  let command = vec!["cargo".to_owned(), "test".to_owned()];
  let placed = PlacedRun::stand(
    &PlaceAsk {
      command: &command,
      adapter: None,
      lane: Lane::Wide,
      cwd: dir.clone(),
      env: &env,
    },
    &dir,
    "machine-2",
  );
  let bench = ReadyBench {
    path: dir.to_string_lossy().into_owned(),
    checkouts: Vec::new(),
    heads: Vec::new(),
    within: String::new(),
  };
  // A stand-in `ssh` whose far side says its turn came and ends, whatever line it is handed.
  install(
    &format!("#!/bin/sh\necho \"{}\" >&2\nexit 4\n", LANES.placed.turn_came),
    &dir.join("ssh"),
  );

  let wide = lock_path(&env, Lane::Wide);

  assert_eq!(read_placed(&wide)[0].entry.since, None);
  assert_eq!(placement.run(&bench, &command, false, Some(&placed), &Default::default()), 4);
  assert!(read_placed(&wide)[0].entry.since.is_some());
}

// "a command that writes nothing for the bench's silence limit is ended whole and the verb exits 124 with the
// hung line": the declared limit of a second, the command's output passed on as it came, and what it left
// standing gone with it.
#[test]
fn the_placed_verb_ends_a_command_silent_for_the_declared_limit_whole() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let repo = dir.join("repo");
  let left = dir.join("left");
  let line = format!("echo once; echo said >&2; sleep 30 & echo $! > {}; wait", left.display());

  checkout(&repo, "testbed = 0\n\n[test]\nsilence_seconds = 1\n");

  let done = ran(&mut testbed(&repo, &dir, &["placed", "--", "sh", "-c", &line]));
  let hung = fill_marks(
    &LANES.silence.hung,
    &[
      ("command", &format!("sh -c {line}")),
      ("output", &LANES.silence.piped),
      ("seconds", "1"),
      ("standing", &fill_marks(&LANES.silence.standing, &[("processes", "sleep 30")])),
    ],
  );
  let pid: i64 = std::fs::read_to_string(&left).unwrap().trim().parse().unwrap();

  assert_eq!(done.code, 124, "{}\n{}", done.out, done.err);
  assert_eq!(done.out, "once\n");
  assert_eq!(done.err, format!("said\n{hung}\n"));
  assert!(!process_alive(pid));
}

// "a command that computes without a word is working", and "what the `placed` verb hears without a byte passing
// it says over the connection": a command writing nothing, whose process under it spins past the declared limit of
// a second until the test lets it end, ends on its own code, and the line saying it still works came meanwhile.
#[test]
fn the_placed_verb_keeps_a_working_command_and_says_it_works_over_the_connection() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let repo = dir.join("repo");
  let done = dir.join("done");
  let line = format!("sh -c 'while [ ! -e {} ]; do :; done' & wait", done.display());

  checkout(&repo, "testbed = 0\n\n[test]\nsilence_seconds = 1\n");

  let ended = std::thread::scope(|scope| {
    scope.spawn(|| {
      std::thread::sleep(Duration::from_millis(2500));
      std::fs::write(&done, "").unwrap();
    });
    ran(testbed(&repo, &dir, &["placed", "--", "sh", "-c", &line]).env(STANDING_VARIABLE, "200"))
  });
  let working = LANES.silence.working.split("{seconds}").next().unwrap();
  let working = fill_marks(working, &[("command", &format!("sh -c {line}"))]);

  assert_eq!(ended.code, 0, "{}\n{}", ended.out, ended.err);
  assert!(ended.err.lines().any(|said| said.starts_with(&working)), "{}", ended.err);
}

// "the setup steps and the command pass their output on through pipes it counts": while the command writes, the
// file the verb's own holder handed it in `BMO_TESTBED_HEARD` is touched.
#[test]
fn the_placed_verb_tells_its_own_holder_while_its_command_writes() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let repo = dir.join("repo");
  let heard = dir.join("heard");
  let long_ago = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(3600);

  checkout(&repo, "testbed = 0\n");
  std::fs::File::create(&heard).unwrap().set_modified(long_ago).unwrap();

  let done = ran(
    testbed(&repo, &dir, &["placed", "--", "sh", "-c", "echo written"]).env("BMO_TESTBED_HEARD", &heard),
  );

  assert_eq!((done.code, done.out.as_str()), (0, "written\n"), "{}", done.err);
  assert!(std::fs::metadata(&heard).unwrap().modified().unwrap() > long_ago);
}

// A stand-in `ssh` whose far side is a home of its own under the directory given, with a `systemctl` there
// that writes down each line it is asked and answers 0.
fn far_home(dir: &Path) -> std::path::PathBuf {
  let ssh = dir.join("far-ssh");
  let bin = dir.join("far-bin");

  std::fs::create_dir_all(&bin).unwrap();
  install(
    &format!(
      "#!/bin/sh\nexport HOME='{home}'\nexport PATH='{bin}':\"$PATH\"\nexec '{ssh}' \"$@\"\n",
      home = dir.join("far-home").display(),
      bin = bin.display(),
      ssh = dir.join("ssh").display(),
    ),
    &ssh,
  );
  install(SSH, &dir.join("ssh"));
  install(
    &format!("#!/bin/sh\necho \"$*\" >> '{}'\n", dir.join("systemctl.log").display()),
    &bin.join("systemctl"),
  );
  ssh
}

// "written whole under that machine's ~/.config/systemd/user/ and renamed over whatever stood at its name
// there, then that machine's user service manager reloaded and each laid timer enabled and started"
#[test]
fn the_units_marked_for_the_machine_are_laid_in_its_user_units_and_their_timers_started_there() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let mut machine = released(&dir, None, None);
  let units_there = dir.join("far-home/.config/systemd/user");
  let units = [
    ("sweep.timer".to_owned(), "[Timer]\nOnCalendar=daily\n".to_owned()),
    ("sweep.service".to_owned(), "[Service]\nType=oneshot\n".to_owned()),
  ];

  machine.settings.ssh = vec![far_home(&dir).to_string_lossy().into_owned()];
  std::fs::create_dir_all(&units_there).unwrap();
  std::os::unix::fs::symlink(dir.join("elsewhere.timer"), units_there.join("sweep.timer")).unwrap();

  assert_eq!(machine.lay_units(&units, &["sweep.timer".to_owned()]), Ok(()));

  for (name, text) in &units {
    let at = units_there.join(name);

    assert!(!std::fs::symlink_metadata(&at).unwrap().file_type().is_symlink(), "{name}");
    assert_eq!(&std::fs::read_to_string(at).unwrap(), text);
  }

  assert_eq!(
    std::fs::read_to_string(dir.join("systemctl.log")).unwrap(),
    "--user daemon-reload\n--user enable --now sweep.timer\n"
  );

  let refusing = dir.join("refusing-ssh");

  install("#!/bin/sh\necho 'no route to stand' >&2\nexit 255\n", &refusing);
  machine.settings.ssh = vec![refusing.to_string_lossy().into_owned()];

  assert_eq!(machine.lay_units(&units, &[]), Err("no route to stand".to_owned()));
}

// "disk_free_gigabytes: 50, the prune policy's ... Not negative"
#[test]
fn the_disk_floor_is_the_policy_s_unless_the_settings_name_one_and_a_negative_one_does_not_read() {
  let dir = tempfile::tempdir().unwrap();
  let settings = |text: &str| {
    std::fs::create_dir_all(dir.path().join("env")).unwrap();
    std::fs::write(dir.path().join("env/placement.toml"), text).unwrap();
    MachineSettings::read(dir.path()).unwrap()
  };
  let named =
    settings("destination = \"stand\"\ndisk_free_gigabytes = 2.5\ndisk_paths = [\"/mnt/c\"]\n").unwrap();

  assert_eq!(named.disk_floor_bytes(), 2_500_000_000);
  assert_eq!(named.disk_paths(), ["~/.local/share/bmo/cargo-target", "/mnt/c"]);
  assert_eq!(
    settings("destination = \"stand\"\n").unwrap().disk_floor_bytes(),
    (PRUNE.placed_disk_free_gigabytes * 1e9) as u64
  );
  assert!(
    settings("destination = \"stand\"\ndisk_free_gigabytes = -1\n")
      .unwrap_err()
      .contains(&LANES.placed.negative_floor)
  );
}

// "the settings put the machines in order, the placed machine by its label and this machine as `local`; none
// named is the placed machine, then this one; an empty order, a machine named twice or one that is neither does
// not read"
#[test]
fn the_order_reads_by_name_and_refuses_an_empty_a_repeated_or_an_unknown_machine() {
  let dir = tempfile::tempdir().unwrap();
  let settings = |order: &str| {
    std::fs::create_dir_all(dir.path().join("env")).unwrap();
    std::fs::write(
      dir.path().join("env/placement.toml"),
      format!("machine = \"machine-2\"\ndestination = \"stand\"\n{order}"),
    )
    .unwrap();
    MachineSettings::read(dir.path()).unwrap().map(|settings| settings.order().unwrap())
  };
  let here = &LANES.placed.here;

  assert_eq!(settings("").unwrap(), [Ordered::Placed, Ordered::Here]);
  assert_eq!(
    settings(&format!("order = [\"{here}\", \"machine-2\"]\n")).unwrap(),
    [Ordered::Here, Ordered::Placed]
  );
  assert_eq!(settings("order = [\"machine-2\"]\n").unwrap(), [Ordered::Placed]);
  assert!(settings("order = []\n").unwrap_err().contains(&LANES.placed.order_empty));
  assert!(
    settings(&format!("order = [\"{here}\", \"{here}\"]\n"))
      .unwrap_err()
      .contains(&fill_marks(&LANES.placed.order_twice, &[("name", here)]))
  );
  assert!(settings("order = [\"stand\"]\n").unwrap_err().contains(&fill_marks(
    &LANES.placed.order_unknown,
    &[("name", "stand"), ("machine", "machine-2"), ("here", here)]
  )));
}

// "A file that names no `destination` pauses placement. It reads as no placed machine ... The rest of the file is
// read all the same ... `order` and `combined_order` may name `local` alone"; "`placement` ... `paused`": a machine
// taken out with its lines commented out, as the settings file keeps it.
#[test]
fn a_file_naming_no_destination_pauses_placement_and_reads_the_rest() {
  let dir = tempfile::tempdir().unwrap();
  let here = &LANES.placed.here;
  let write = |text: &str| {
    std::fs::create_dir_all(dir.path().join("env")).unwrap();
    std::fs::write(dir.path().join("env/placement.toml"), text).unwrap();
  };

  write(&format!(
    "# machine = \"machine-2\"\n# destination = \"stand\"\ndisk_paths = [\"/mnt/c\"]\norder = [\"{here}\"]\n"
  ));

  let paused = MachineSettings::read(dir.path()).unwrap().unwrap();

  assert!(paused.paused());
  assert_eq!(paused.disk_paths(), ["~/.local/share/bmo/cargo-target", "/mnt/c"]);
  assert_eq!(paused.order().unwrap(), [Ordered::Here]);
  assert_eq!(paused.combined_order().unwrap(), [Ordered::Here], "none named is this machine alone");
  assert!(MachinePlacement::named(dir.path(), dir.path()).is_none(), "no machine is placed");
  assert_eq!(PlacementState::of(dir.path()), PlacementState::Paused);
  assert!(
    PlacementState::of(dir.path())
      .line(dir.path())
      .is_some_and(|line| line.contains("placement.toml"))
  );

  // Naming the machine that is out is refused, and the rest of the file still has to read.
  write("machine = \"machine-2\"\norder = [\"machine-2\"]\n");
  assert!(
    MachineSettings::read(dir.path())
      .unwrap()
      .unwrap_err()
      .contains(&fill_marks(&LANES.placed.order_paused, &[("name", "machine-2"), ("here", here)]))
  );
  assert_eq!(PlacementState::of(dir.path()), PlacementState::Unread);
  write("disk_free_gigabytes = -1\n");
  assert!(MachineSettings::read(dir.path()).unwrap().is_err());
}

// A turn held in one lane of the seats a lock names, in a machine of one wide seat.
fn holding(lock: &Path, lane: Lane) -> TestbedTurn {
  take_turn(&TurnAsk {
    command: "a holder".to_owned(),
    lane,
    say: &|_| {},
    path: None,
    env: Env::from([
      ("BMO_TESTBED_LOCK".to_owned(), lock.to_string_lossy().into_owned()),
      ("BMO_TESTBED_SEATS".to_owned(), "1".to_owned()),
    ]),
    checkout: "/checkout".to_owned(),
    poll: None,
    notice_seconds: None,
  })
}

// "a run goes to the first machine in the order with a seat of its lane free now, and falls to the next only
// where every seat there is held or waited for; the last takes it whatever it holds": the placed machine, whose
// lanes stand apart from this one's, first, then this machine first.
#[test]
fn a_run_goes_to_the_first_machine_in_the_order_with_a_seat_free_and_falls_to_the_next_when_it_is_full() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let repo = dir.join("repo");
  let ssh = dir.join("ssh");
  let far_lock = dir.join("far.lock");
  let order = |names: &str| {
    settle(&dir, &ssh, "stand");
    let file = dir.join("env/placement.toml");
    let text = std::fs::read_to_string(&file).unwrap();

    std::fs::write(&file, format!("{text}order = {names}\n")).unwrap();
  };
  let run = |narrow: bool| {
    let mut args = vec!["run"];

    if narrow {
      args.push("--narrow");
    }

    args.extend(["--adapter", "plain", "--", "sh", "-c", "exit 7"]);
    ran(testbed(&repo, &dir, &args).env("HOME", &dir).env("BMO_TESTBED_SEATS", "1"))
  };
  let bench = dir.join("bench/repo");
  let running =
    fill_marks(&LANES.placed.running, &[("machine", "machine-2"), ("bench", &bench.to_string_lossy())]);
  let full =
    fill_marks(&LANES.placed.full, &[("machine", "machine-2"), ("why", &LANES.placed.seats_taken)]);

  // The far side's turns stand at a lock of their own, as another machine's do.
  install(
    &SSH.replace("exec sh -c", &format!("export BMO_TESTBED_LOCK={}; exec sh -c", far_lock.display())),
    &ssh,
  );
  checkout(&repo, "testbed = 0\n");

  order(&format!("[\"machine-2\", \"{}\"]", LANES.placed.here));

  let far_narrow = holding(&far_lock, Lane::Narrow);
  let there = run(false);
  let fell = run(true);

  drop(far_narrow);
  assert_eq!(there.code, 7, "{}\n{}", there.out, there.err);
  assert!(there.err.contains(&running), "{}", there.err);
  assert_eq!(fell.code, 7, "{}\n{}", fell.out, fell.err);
  assert!(fell.err.contains(&full), "{}", fell.err);
  assert!(!fell.err.contains(&running), "{}", fell.err);

  order(&format!("[\"{}\", \"machine-2\"]", LANES.placed.here));

  let here = run(false);
  let local_wide = holding(&dir.join("testbed.lock"), Lane::Wide);
  let placed = run(false);

  drop(local_wide);
  assert_eq!(here.code, 7, "{}\n{}", here.out, here.err);
  assert!(!here.err.contains(&running), "{}", here.err);
  assert_eq!(placed.code, 7, "{}\n{}", placed.out, placed.err);
  assert!(placed.err.contains(&running), "{}", placed.err);
}

// "the placed machine answers for its own through `status --free`, whose line saying why it takes no run the
// caller passes on ... a loaded machine takes no run wherever it stands in the order, and a run no machine of
// the order takes runs where it was started"
#[test]
fn a_loaded_machine_takes_no_run_wherever_it_stands_and_a_run_none_takes_runs_where_it_stands() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let repo = dir.join("repo");
  let calm = dir.join("ssh");
  let busy = dir.join("busy-ssh");
  let here = &LANES.placed.here;
  let place = |ssh: &Path, order: &str, share: &str| {
    settle(&dir, ssh, "stand");
    let file = dir.join("env/placement.toml");
    let text = std::fs::read_to_string(&file).unwrap();

    std::fs::write(&file, format!("{text}order = {order}\n")).unwrap();
    ran(
      testbed(&repo, &dir, &["run", "--adapter", "plain", "--", "sh", "-c", "exit 7"])
        .env("HOME", &dir)
        .env(LOADED_VARIABLE, share),
    )
  };
  let running = fill_marks(
    &LANES.placed.running,
    &[("machine", "machine-2"), ("bench", &dir.join("bench/repo").to_string_lossy())],
  );
  let passed_on = format!(
    "{}{}",
    LANES.placed.full.split("{why}").next().unwrap().replace("{machine}", "machine-2"),
    LANES.placed.loaded.split("{load}").next().unwrap()
  );

  install(SSH, &calm);
  // The far side's own share of 0 makes it loaded whatever its load.
  install(&SSH.replace("exec sh -c", "export BMO_TESTBED_LOADED_PERCENT=0; exec sh -c"), &busy);
  checkout(&repo, "testbed = 0\n");

  let fell = place(&busy, &format!("[\"machine-2\", \"{here}\"]"), "1000000");

  assert_eq!(fell.code, 7, "{}\n{}", fell.out, fell.err);
  assert!(fell.err.contains(&passed_on), "{}", fell.err);
  assert!(!fell.err.contains(&running), "{}", fell.err);

  let sent = place(&calm, &format!("[\"{here}\", \"machine-2\"]"), "0");

  assert_eq!(sent.code, 7, "{}\n{}", sent.out, sent.err);
  assert!(sent.err.contains(&running), "{}", sent.err);

  // The same machine stands last in the order and turns the run away there too, so it is this machine's.
  let last = place(&busy, &format!("[\"{here}\", \"machine-2\"]"), "0");

  assert_eq!(last.code, 7, "{}\n{}", last.out, last.err);
  assert!(last.err.contains(&passed_on), "{}", last.err);
  assert!(!last.err.contains(&running), "{}", last.err);
}

// "a failing log a placed run keeps there is brought here": a far run that fails naming, under a repository's
// name on its standard output, a failing log kept there has it copied under this machine's data root and said on
// a line; one naming, on its standard error, a log that does not stand there is said not brought; the run's code
// stays the one it ended with there
#[test]
fn a_failing_log_kept_on_the_placed_machine_is_brought_here_and_named() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let repo = dir.join("repo");
  let ssh = dir.join("ssh");
  let far = dir.join("far/check-alpha-1234.failed.log");
  let missing = dir.join("far/check-set-5678.failed.log");
  let kept = |path: &Path| fill_marks(&SUITE.check.kept, &[("path", &path.to_string_lossy())]);

  std::fs::create_dir_all(far.parent().unwrap()).unwrap();
  std::fs::write(&far, "==> tests: exit 1\nthe whole of what failed\n").unwrap();
  install(
    &SSH.replacen(
      "shift\nexec",
      &format!(
        "shift\ncase \"$*\" in *\" placed \"*) echo 'alpha: {}'; echo '{}' >&2; exit 1 ;; esac\nexec",
        kept(&far),
        kept(&missing)
      ),
      1,
    ),
    &ssh,
  );
  settle(&dir, &ssh, "stand");
  checkout(&repo, "testbed = 0\n");

  let done = on_machine(&repo, &dir, &["run", "--adapter", "plain", "--", "sh", "-c", "exit 1"]);
  let copy = dir.join("testbed/placed/machine-2/check-alpha-1234.failed.log");
  let brought = fill_marks(
    &LANES.placed.log_brought,
    &[("machine", "machine-2"), ("path", &copy.to_string_lossy())],
  );
  let unbrought_head = fill_marks(
    LANES.placed.log_unbrought.split("{said}").next().unwrap(),
    &[("machine", "machine-2"), ("far", &missing.to_string_lossy())],
  );

  assert_eq!(done.code, 1, "{}\n{}", done.out, done.err);
  assert!(done.out.contains(&kept(&far)), "the kept line is passed on: {}", done.out);
  assert!(done.err.contains(&brought), "{}", done.err);
  assert_eq!(std::fs::read_to_string(&copy).unwrap(), std::fs::read_to_string(&far).unwrap());
  assert!(done.err.contains(&unbrought_head), "{}", done.err);
}

// A hook that writes down the name and the payload of each event it is handed.
fn writing_hook(dir: &Path) -> Hook {
  let script = dir.join("hook");

  install(
    &format!(
      "#!/bin/sh\necho \"$1\" >> '{names}'\ncat >> '{payloads}'\necho >> '{payloads}'\n",
      names = dir.join("events").display(),
      payloads = dir.join("payloads").display(),
    ),
    &script,
  );
  Hook {
    command: Some(script.to_string_lossy().into_owned()),
    time: std::time::Duration::from_secs(60),
    repository: None,
    root: None,
  }
}

// "A disk below the floor sends placed-disk-low ... The event names the machine, the floor, and each disk below
// it by its path, its source ... and its free space"; "A path whose free space does not read is said on a line";
// "A dry run reads and says the free space and sends nothing"
#[test]
fn a_disk_of_the_placed_machine_below_its_floor_sends_placed_disk_low_and_one_above_it_sends_nothing() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let mut machine = released(&dir, None, None);
  let target = machine.settings.cargo_target.clone();
  let gone = dir.join("gone").to_string_lossy().into_owned();
  let hook = writing_hook(&dir);
  let said = &VERBS.prune;

  std::fs::create_dir_all(&target).unwrap();
  machine.settings.disk_paths = vec![gone.clone()];
  machine.settings.disk_free_gigabytes = Some(0.0);

  let above = placed_disk(&machine, true, &hook);

  assert!(above[1].starts_with(&format!("  {target} on ")), "{above:?}");
  assert!(!above[1].contains("below"), "{above:?}");
  assert_eq!(above[2], fill_marks(&said.placed_unread, &[("path", &gone)]));
  assert!(!dir.join("events").exists());

  // A floor no disk reaches.
  machine.settings.disk_free_gigabytes = Some(1e9);

  let dry = placed_disk(&machine, false, &hook);

  assert!(dry[1].ends_with(" free, below the floor"), "{dry:?}");
  assert!(!dir.join("events").exists());

  placed_disk(&machine, true, &hook);

  let payload: Value =
    serde_json::from_str(std::fs::read_to_string(dir.join("payloads")).unwrap().trim()).unwrap();

  assert_eq!(std::fs::read_to_string(dir.join("events")).unwrap(), "placed-disk-low\n");
  assert_eq!(payload["detail"]["machine"], "machine-2");
  assert_eq!(payload["detail"]["floor_bytes"], machine.settings.disk_floor_bytes());
  assert_eq!(payload["detail"]["disks"].as_array().unwrap().len(), 1);
  assert_eq!(payload["detail"]["disks"][0]["path"], target.as_str());
  assert!(payload["detail"]["disks"][0]["free_bytes"].as_u64().unwrap() > 0);
  assert!(!payload["detail"]["disks"][0]["source"].as_str().unwrap().is_empty());
}

// "Where the machine's disk is a virtual one inside a host drive, as a WSL machine's is, the settings'
// disk_paths names that drive's mount ... The host drive's figure is therefore the one that warns of a full
// host. The virtual disk's own figure is read beside it"
#[test]
fn a_host_drive_below_the_floor_warns_by_its_drive_and_figure_while_the_virtual_disk_has_room() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let mut machine = released(&dir, None, None);
  let answering = dir.join("answering-ssh");
  let hook = writing_hook(&dir);

  // `findmnt`'s answers for the virtual disk holding the build root and for the host drive's mount, in order.
  install(
    "#!/bin/sh\n\
     printf '%s\\n' '{\"filesystems\": [{\"source\": \"/dev/sdd\", \"avail\": 435000000000}]}'\n\
     printf '%s\\n' '{\"filesystems\": [{\"source\": \"C:\\\\\", \"avail\": \"12000000000\"}]}'\n",
    &answering,
  );
  machine.settings.ssh = vec![answering.to_string_lossy().into_owned()];
  machine.settings.disk_paths = vec!["/mnt/c".to_owned()];
  machine.settings.disk_free_gigabytes = Some(50.0);

  let lines = placed_disk(&machine, true, &hook);
  let target = machine.settings.cargo_target.clone();
  let payload: Value =
    serde_json::from_str(std::fs::read_to_string(dir.join("payloads")).unwrap().trim()).unwrap();

  assert_eq!(lines[1], format!("  {target} on /dev/sdd: 435.0 GB free"));
  assert_eq!(lines[2], "  /mnt/c on C:\\: 12.0 GB free, below the floor");
  assert_eq!(
    payload["summary"],
    "The placed machine machine-2 has less than 50.0 GB free on C:\\ at /mnt/c with 12.0 GB free."
  );
  assert_eq!(
    payload["detail"]["disks"],
    serde_json::json!([{ "path": "/mnt/c", "source": "C:\\", "free_bytes": 12_000_000_000_u64 }])
  );
}

// "a machine that cannot be reached is said on one line, its disk not read"
#[test]
fn a_placed_machine_out_of_reach_has_its_disk_said_unread_and_sends_nothing() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let mut machine = released(&dir, None, None);
  let refusing = dir.join("refusing-ssh");
  let hook = writing_hook(&dir);

  install("#!/bin/sh\necho 'no route to stand' >&2\nexit 255\n", &refusing);
  machine.settings.ssh = vec![refusing.to_string_lossy().into_owned()];
  machine.settings.disk_free_gigabytes = Some(1e9);

  let lines = placed_disk(&machine, true, &hook);

  assert_eq!(
    lines[1..],
    [fill_marks(&VERBS.prune.placed_unreached, &[("said", "no route to stand")])]
  );
  assert!(!dir.join("events").exists());
}

// A stand-in `ssh` that runs its line on this machine as `SSH` does, in a home and a data root of the far
// machine's own under the directory given, standing for no harness and inside no turn; the first `drops` lines
// running the `placed` verb end as a connection that broke does, with `ssh`'s own 255.
fn dropping_ssh(dir: &Path, drops: usize) -> String {
  let far = dir.join("far-home");

  format!(
    r#"#!/bin/sh
while [ $# -gt 0 ]; do
  case "$1" in
    -o|-p|-i|-l|-F) shift 2 ;;
    -*) shift ;;
    *) break ;;
  esac
done
shift
case "$*" in
  *" placed "*)
    n=$(( $(cat {count} 2>/dev/null || echo 0) + 1 ))
    echo $n > {count}
    if [ $n -le {drops} ]; then echo 'client_loop: send disconnect: Broken pipe' >&2; exit 255; fi ;;
esac
mkdir -p {far}/data
export HOME={far} BMO_DATA_DIR={far}/data BMO_TESTBED_LOCK={far}/data/testbed.lock XDG_RUNTIME_DIR={far}/data \
  BMO_TESTBED_LOADED_PERCENT=1000000
unset CLAUDE_PID BMO_TESTBED_SCOPE BMO_TESTBED_SERVICE BMO_TESTBED_TOKEN BMO_TESTBED_HELD BMO_TESTBED_FRONT \
  BMO_TESTBED_NEXT BMO_TESTBED_PLACEMENT BMO_TESTBED_PLACED BMO_TESTBED_CHECKOUTS BMO_DEPLOY_UNIT \
  DBUS_SESSION_BUS_ADDRESS
exec sh -c "$*"
"#,
    count = dir.join("placed-lines").display(),
    far = far.display(),
  )
}

// A deployment's gate on the placed machine whose `ssh` drops the first `drops` runs there, its testbed this
// binary, over a checkout whose static check is one step that passes; its order names the placed machine alone,
// so broken work is tried there again rather than coming here.
fn dropping_gate(dir: &Path, drops: usize) -> PlacedGate<'static> {
  let mut machine = released(dir, None, None);
  let ssh = dir.join("ssh");

  install(&dropping_ssh(dir, drops), &ssh);
  machine.settings.order = Some(vec!["machine-2".to_owned()]);
  machine.settings.ssh = vec![ssh.to_string_lossy().into_owned()];
  machine.settings.testbed = env!("CARGO_BIN_EXE_bmo-testbed").to_owned();
  checkout(
    &dir.join("repo"),
    "testbed = 0\n\n[[check.step]]\nid = \"passes\"\ncommand = \"true\"\n",
  );

  bringing(machine, false)
}

// The gate asked of the scene's checkout at its `HEAD`, its suite left out.
fn passed_at(gate: &PlacedGate<'_>, dir: &Path) -> bmo_testbed_deploy::deployment::GateOutcome {
  let repo = dir.join("repo");
  let commit = git(&repo, &["rev-parse", "HEAD"]);

  gate.pass(&GateAsk {
    root: &repo,
    tree: &repo,
    commit: &commit,
    since: None,
    suite: false,
    heads: &[],
    declaration: &Declaration::default(),
    earlier: &none_earlier,
  })
}

// "The connection counts as broken where `ssh` ends with its own code, 255 ... and runs that work again from its
// bench"
#[test]
fn a_static_check_the_connection_breaks_under_answers_broken_and_passes_when_asked_again() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  // The first two lines there break: the toolchains' read, which leaves the gate on no footing, and the static
  // check.
  let gate = dropping_gate(&dir, 2);
  let broke = fill_marks(&LANES.placed.broke, &[("machine", "machine-2")]);

  let first = passed_at(&gate, &dir);

  assert!(!first.passed && first.broken, "{first:?}");
  assert_eq!(first.reason, broke);
  assert_eq!(first.steps[0].code, i64::from(NOT_RUN_EXIT_CODE));

  let again = passed_at(&gate, &dir);

  assert!(again.passed && !again.broken, "{again:?}");
  assert_eq!(again.steps[0].code, 0);
}

// "Where the placement order names this machine, the work comes here instead of waiting ... the whole gate as
// this machine's own, or the build in this machine's build directory"
#[test]
fn a_deployment_s_work_comes_here_where_the_order_names_this_machine_and_its_placed_machine_is_away() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let refusing = dir.join("refusing-ssh");
  let repo = dir.join("repo");
  let mut gate = dropping_gate(&dir, 0);

  install("#!/bin/sh\necho 'no agent named stand is attached' >&2\nexit 255\n", &refusing);
  gate.machine.settings.order = Some(vec!["machine-2".to_owned(), LANES.placed.here.clone()]);
  gate.machine.settings.ssh = vec![refusing.to_string_lossy().into_owned()];

  let judged = passed_at(&gate, &dir);

  assert!(judged.passed && !judged.broken, "the gate is this machine's own: {judged:?}");

  let built = gate.build(&BuildAsk {
    root: &repo,
    commit: &git(&repo, &["rev-parse", "HEAD"]),
    tree: &repo,
    profile: "release",
    binaries: &[],
    built: &dir.join("built"),
  });

  assert_eq!(built, None, "the build runs here");
}

// "Where the order does not name this machine, a try waits for the placed machine. A try that finds the machine
// not answering is one more broken try, whether or not the deployment has begun there; nothing moves to the
// machine the deployment runs on"
#[test]
fn a_machine_out_of_reach_answers_broken_whether_or_not_begun_there_and_nothing_runs_here() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let refusing = dir.join("refusing-ssh");
  let broke = fill_marks(&LANES.placed.broke, &[("machine", "machine-2")]);
  let build = |gate: &PlacedGate<'_>, at: &Path| {
    let repo = at.join("repo");

    gate.build(&BuildAsk {
      root: &repo,
      commit: &git(&repo, &["rev-parse", "HEAD"]),
      tree: &repo,
      profile: "release",
      binaries: &[],
      built: &at.join("built"),
    })
  };
  let (fresh_at, begun_at) = (dir.join("fresh"), dir.join("begun"));

  install("#!/bin/sh\necho 'no agent named stand is attached' >&2\nexit 255\n", &refusing);
  std::fs::create_dir_all(&fresh_at).unwrap();
  std::fs::create_dir_all(&begun_at).unwrap();

  let mut fresh = dropping_gate(&fresh_at, 0);

  fresh.machine.settings.ssh = vec![refusing.to_string_lossy().into_owned()];

  let unreached_fresh = passed_at(&fresh, &fresh_at);

  assert!(!unreached_fresh.passed && unreached_fresh.broken, "{unreached_fresh:?}");
  assert_eq!(unreached_fresh.reason, broke);
  assert_eq!(build(&fresh, &fresh_at), Some(Err(GateFault::Broken(broke.clone()))));

  let mut begun = dropping_gate(&begun_at, 0);

  assert!(passed_at(&begun, &begun_at).passed);

  begun.machine.settings.ssh = vec![refusing.to_string_lossy().into_owned()];

  let unreached = passed_at(&begun, &begun_at);

  assert!(!unreached.passed && unreached.broken, "{unreached:?}");
  assert_eq!(unreached.reason, broke);
  assert_eq!(build(&begun, &begun_at), Some(Err(GateFault::Broken(broke))));
}

// The stand-in `ssh` with every `placed` line written down, one a line, and ended there; `status --free`
// answers with `free`, the code for a reason the machine gives, so a run that may fall to this machine would.
fn set_ssh(dir: &Path, free: &str) -> (PathBuf, PathBuf) {
  let ssh = dir.join("ssh");
  let lines = dir.join("placed-lines");

  install(
    &SSH.replacen(
      "shift\nexec",
      &format!(
        "shift\ncase \"$*\" in *\" placed \"*) echo \"$*\" >> {}; exit 0 ;; *\"status --free\"*) {free} ;; esac\nexec",
        lines.display()
      ),
      1,
    ),
    &ssh,
  );
  (ssh, lines)
}

// "a change check over several checkouts is placed as one set: every checkout laid in one bench at the place it
// has under this machine's home, the set run there by one `ssh` command naming each checkout by its path from
// the first's bench and its since by the commit, and by default waiting in the placed machine's line however
// many of its seats are held"
#[test]
fn a_set_of_checkouts_is_placed_as_one_run_laid_as_here_and_waits_in_a_full_placed_machine() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let alpha = dir.join("work/alpha");
  let beta = dir.join("base/beta");
  let (ssh, lines) = set_ssh(&dir, "exit 1");

  settle(&dir, &ssh, "stand");
  checkout(&alpha, "testbed = 0\n");
  checkout(&beta, "testbed = 0\n");
  std::fs::write(alpha.join("open.txt"), "not yet committed").unwrap();
  git(&beta, &["branch", "base"]);

  let beta_base = git(&beta, &["rev-parse", "base"]);
  let alpha_head = git(&alpha, &["rev-parse", "HEAD"]);
  let done = on_machine(&alpha, &dir, &["check", "--checkout", ".", "--checkout", "../../base/beta@base"]);
  let handed = std::fs::read_to_string(&lines).expect("the set was placed");
  let handed: Vec<&str> = handed.lines().collect();

  assert_eq!(done.code, 0, "{}\n{}", done.out, done.err);
  assert_eq!(handed.len(), 1, "{handed:?}");
  assert!(
    handed[0].contains(&format!("cd {}", dir.join("bench/work/alpha").display())),
    "{}",
    handed[0]
  );
  let first = handed[0]
    .find(&format!(".@{alpha_head}"))
    .expect("the first checkout is named from its bench");
  let second = handed[0]
    .find(&format!("../../base/beta@{beta_base}"))
    .expect("the second is named from the first's bench");

  assert!(first < second, "{}", handed[0]);
  assert!(
    handed[0].contains(&format!("--checkout {}", dir.join("bench/base/beta").display())),
    "{}",
    handed[0]
  );
  assert!(!done.out.contains("check:"), "the set ran here: {}", done.out);

  for (bench, from) in [(dir.join("bench/work/alpha"), &alpha), (dir.join("bench/base/beta"), &beta)] {
    assert_eq!(git(&bench, &["rev-parse", "HEAD"]), git(from, &["rev-parse", "HEAD"]));
  }

  assert_eq!(
    std::fs::read_to_string(dir.join("bench/work/alpha/open.txt")).unwrap(),
    "not yet committed"
  );
}

// "a loaded machine takes no run wherever it stands in the order, and a run no machine of the order takes runs
// where it was started": the set whose placed machine answers loaded is checked here, though the settings name
// no other machine for it
#[test]
fn a_set_turns_away_a_loaded_placed_machine_and_is_checked_where_it_was_started() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let alpha = dir.join("alpha");
  let beta = dir.join("beta");
  let loaded = i32::from(LOADED_EXIT_CODE).to_string();
  let (ssh, lines) = set_ssh(&dir, &format!("exit {loaded}"));

  settle(&dir, &ssh, "stand");
  checkout(&alpha, "testbed = 0\n");
  checkout(&beta, "testbed = 0\n");

  let done = on_machine(&alpha, &dir, &["check", "--checkout", ".", "--checkout", "../beta"]);

  assert_eq!(done.code, 0, "{}\n{}", done.out, done.err);
  assert!(checked_here(&done), "{}", done.out);
  assert!(!lines.exists(), "a loaded machine was handed the set");
}

// Whether both checkouts of the set were checked here, by the lines naming each one's check.
fn checked_here(done: &Ran) -> bool {
  ["alpha: check:", "beta: check:"]
    .iter()
    .all(|head| done.out.lines().any(|line| line.starts_with(head)))
}

// "a testbed whose settings name no placed machine runs the set here, and so does a set kept here where no placed
// machine is named; every other placement refuses it: the endpoint, and a run a declared variable keeps here while
// a placed machine is named, unless that machine's combined_order names this machine"
#[test]
fn a_set_runs_here_where_no_placed_machine_is_named_and_is_refused_where_one_would_take_it() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let alpha = dir.join("alpha");
  let beta = dir.join("beta");
  let (ssh, lines) = set_ssh(&dir, "exit 1");
  let set = ["check", "--checkout", ".", "--checkout", "../beta"];

  checkout(&alpha, "testbed = 0\n\n[runner]\nlocal_variables = [\"STAND_LOCAL\"]\n");
  checkout(&beta, "testbed = 0\n");

  let unplaced = on_machine(&alpha, &dir, &set);

  assert_eq!(unplaced.code, 0, "{}\n{}", unplaced.out, unplaced.err);
  assert!(checked_here(&unplaced), "{}", unplaced.out);

  let kept_unplaced = ran(testbed(&alpha, &dir, &set).env("HOME", &dir).env("STAND_LOCAL", "1"));

  assert_eq!(kept_unplaced.code, 0, "{}\n{}", kept_unplaced.out, kept_unplaced.err);
  assert!(checked_here(&kept_unplaced), "{}", kept_unplaced.out);

  settle(&dir, &ssh, "stand");

  let endpoint = ran(
    testbed(&alpha, &dir, &set)
      .env("HOME", &dir)
      .env("BMO_TESTBED_PLACEMENT", "http://127.0.0.1:9"),
  );

  assert_eq!(endpoint.code, 1, "{}\n{}", endpoint.out, endpoint.err);
  assert!(endpoint.err.contains(&LANES.placed.set_one_checkout), "{}", endpoint.err);

  let kept = ran(testbed(&alpha, &dir, &set).env("HOME", &dir).env("STAND_LOCAL", "1"));

  assert_eq!(kept.code, 1, "{}\n{}", kept.out, kept.err);
  assert!(
    kept.err.contains(&fill_marks(&LANES.placed.set_kept, &[("named", "STAND_LOCAL")])),
    "{}",
    kept.err
  );
  assert!(!lines.exists(), "a refused set was placed");

  let settings = dir.join("env/placement.toml");
  let named = std::fs::read_to_string(&settings).unwrap() + "combined_order = [\"machine-2\", \"local\"]\n";

  std::fs::write(&settings, named).unwrap();

  let here = ran(testbed(&alpha, &dir, &set).env("HOME", &dir).env("STAND_LOCAL", "1"));

  assert_eq!(here.code, 0, "{}\n{}", here.out, here.err);
  assert!(
    here.err.contains(&fill_marks(&LANES.placed.kept, &[("named", "STAND_LOCAL")])),
    "{}",
    here.err
  );
  assert!(checked_here(&here), "{}", here.out);
}

// "every command run there over `ssh` asks the connection to answer every `alive_seconds` of the lanes policy
// (30), after the settings' own `ssh` options, and `ssh` ends it as broken once `alive_misses` (4) asks in a
// row go unanswered"
#[test]
fn every_command_there_asks_the_connection_to_keep_answering() {
  let dir = tempfile::tempdir().unwrap();
  let mut placement = released(dir.path(), None, None);
  let ssh = dir.path().join("recording-ssh");
  let heard = dir.path().join("ssh-args");

  install(&format!("#!/bin/sh\necho \"$*\" > '{}'\n", heard.display()), &ssh);
  placement.settings.ssh =
    vec![ssh.to_string_lossy().into_owned(), "-o".to_owned(), "BatchMode=yes".to_owned()];

  assert_eq!(placement.answers(), Ok(()));

  let numbers = &LANES.numbers;
  let asked = std::fs::read_to_string(&heard).unwrap();

  assert_eq!(
    asked.trim(),
    format!(
      "-o BatchMode=yes -o ServerAliveInterval={} -o ServerAliveCountMax={} stand true",
      numbers.alive_seconds, numbers.alive_misses
    )
  );
}
