//! A run placed elsewhere (docs/boundary.md, 「Placement」). Through the placement endpoint, against a
//! stand-in endpoint the test serves: an answer of local, or one that does not come or does not read, runs
//! here; an answer of remote makes the bench ready over a stand-in `ssh` that runs its line on this machine,
//! and the run's output and result come back off its stream. On the placed machine the settings name: the
//! bench made ready over the same stand-in, from the remote that machine can reach or empty where it can reach
//! none, the run the testbed's `placed` verb there, a machine that cannot be reached, or settings that do not
//! read, leaving the run here, the shared versions laid over their copy as their main branch holds them, a
//! deployment's bench standing apart from a run's copy of the same tree, a testbed there that is not this one's release refused before any bench is made and a run meeting it kept
//! here, a run whose environment sets a variable its declaration names as this machine's kept here, the
//! run standing in its lane here while it lasts, running once the far side says its turn came, that line read
//! and not passed on, and the `placed` verb there passing its command's output on, telling its own holder
//! while it comes and ending it whole once it falls silent for the declared limit. The units a release marks
//! for the machine laid in its user units and their timers started there, and the prune's reading of its free
//! disk against the settings' floor, sending `placed-disk-low` for each disk below it.

use std::convert::Infallible;
use std::path::Path;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::response::sse::{Event, Sse};
use axum::routing::{get, post};
use bmo_testbed_cli::cargo::placed_disk;
use bmo_testbed_cli::placement::{
  BenchFor, MachinePlacement, MachineSettings, ReadyBench, reachable_remote,
};
use bmo_testbed_core::events::Hook;
use bmo_testbed_core::placement::{PlaceAsk, PlacedRun, Placement, read_placed};
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::lanes::LANES;
use bmo_testbed_core::policy::prune::PRUNE;
use bmo_testbed_core::policy::verbs::VERBS;
use bmo_testbed_core::queue::process_alive;
use bmo_testbed_core::turn::{Env, Lane, NOT_RUN_EXIT_CODE, lock_path};
use serde_json::{Value, json};

use crate::stand::{Ran, checkout, git, install, ran, testbed};

// A stand-in endpoint: what it was asked, and the address it serves on, which ends with the runtime.
struct Endpoint {
  _runtime: tokio::runtime::Runtime,
  base: String,
  asked: Arc<Mutex<Vec<Value>>>,
}

// An endpoint answering `answer` to every run, `{base}` in it standing for its own address, and serving the
// run's stream as the events given.
fn endpoint(answer: Value, events: Vec<(&'static str, String)>) -> Endpoint {
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
          Sse::new(futures_util::stream::iter(
            events
              .into_iter()
              .map(|(name, data)| Ok::<_, Infallible>(Event::default().event(name).data(data))),
          ))
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
// the way git and rsync hand it theirs.
const SSH: &str = r#"#!/bin/sh
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

// "a machine that cannot be reached, or settings that do not read, leave the run here and say so"
#[test]
fn a_machine_out_of_reach_or_settings_that_do_not_read_leave_the_run_here() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let repo = dir.join("repo");
  let refusing = dir.join("refusing-ssh");

  install("#!/bin/sh\necho 'no route to stand' >&2\nexit 255\n", &refusing);
  settle(&dir, &refusing, "stand");
  checkout(&repo, "testbed = 0\n");

  let here = on_machine(&repo, &dir, &["run", "--adapter", "plain", "--", "sh", "-c", "echo here; exit 4"]);
  let unreached =
    fill_marks(&LANES.placed.unreached, &[("machine", "machine-2"), ("said", "no route to stand")]);

  assert_eq!(here.code, 4, "{}\n{}", here.out, here.err);
  assert!(here.err.contains(&unreached), "{}", here.err);
  assert!(!dir.join("bench").exists());

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

// The placed machine reached through the stand-in `ssh`, its testbed under the directory given with the commit
// `far` beside it where one is named, and this testbed's own release `own`.
fn released(dir: &Path, far: Option<&str>, own: Option<&str>) -> MachinePlacement {
  let ssh = dir.join("ssh");
  let bin = dir.join("far/bin");

  install(SSH, &ssh);
  std::fs::create_dir_all(&bin).unwrap();

  if let Some(far) = far {
    std::fs::write(bin.join("built-from"), format!("{far}\n")).unwrap();
  }

  MachinePlacement {
    settings: MachineSettings {
      destination: "stand".to_owned(),
      machine: Some("machine-2".to_owned()),
      bench: dir.join("bench").to_string_lossy().into_owned(),
      deploy_bench: dir.join("deploy-bench").to_string_lossy().into_owned(),
      ssh: vec![ssh.to_string_lossy().into_owned()],
      testbed: bin.join("bmo-testbed").to_string_lossy().into_owned(),
      cargo_target: dir.join("cargo-target").to_string_lossy().into_owned(),
      disk_free_gigabytes: None,
      disk_paths: Vec::new(),
    },
    home: dir.to_path_buf(),
    release: own.map(str::to_owned),
  }
}

const OWN: &str = "1111111111111111111111111111111111111111";
const FAR: &str = "2222222222222222222222222222222222222222";

// "where they differ, the run is not run and the deployment's stage fails, on a line naming the machine and
// both commits"
#[test]
fn a_testbed_there_of_another_release_is_refused_naming_both_commits_and_no_bench_is_made() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let repo = dir.join("repo");

  checkout(&repo, "testbed = 0\n");

  let machine = released(&dir, Some(FAR), Some(OWN));
  let commit = git(&repo, &["rev-parse", "HEAD"]);
  let refused = machine.ready(&repo, &commit, BenchFor::Deployment("repo")).map(|bench| bench.path);
  let stale = fill_marks(
    &LANES.placed.stale,
    &[("machine", "machine-2"), ("far", &FAR[..12]), ("own", &OWN[..12])],
  );

  assert_eq!(refused, Err(stale));
  assert!(!dir.join("bench").exists());
  assert!(!dir.join("deploy-bench").exists());
}

// "where no such file reads there, the run is not run"; "a testbed running from no release compares nothing"
#[test]
fn the_same_release_passes_one_naming_none_is_refused_and_none_here_compares_nothing() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();

  assert_eq!(released(&dir, Some(OWN), Some(OWN)).same_release(), Ok(()));

  std::fs::remove_file(dir.join("far/bin/built-from")).unwrap();

  let unreleased = released(&dir, None, Some(OWN)).same_release().unwrap_err();
  let file = dir.join("far/bin/built-from");
  let opening =
    fill_marks(&LANES.placed.unreleased, &[("machine", "machine-2"), ("file", &file.to_string_lossy())]);

  assert!(unreleased.starts_with(opening.split("{said}").next().unwrap()), "{unreleased}");
  assert!(unreleased.contains(&OWN[..12]), "{unreleased}");
  assert_eq!(released(&dir, None, None).same_release(), Ok(()));
}

// A checkout holding one package, its manifest naming the dependency given by a path, committed.
fn package_checkout(dir: &Path, name: &str, dependency: Option<&str>) {
  checkout(dir, &format!("testbed = 0\nname = \"{name}\"\n"));
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

// "where they differ, a run, a suite, a change check or a Cargo verb runs here instead"
#[test]
fn a_run_meeting_a_testbed_there_of_another_release_is_kept_here_and_no_bench_is_made() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let repo = dir.join("repo");

  checkout(&repo, "testbed = 0\n");

  let command = vec!["true".to_owned()];
  let env = Env::new();
  let ask = PlaceAsk {
    command: &command,
    adapter: Some("plain"),
    lane: Lane::Wide,
    cwd: repo.clone(),
    env: &env,
    to_free_seat: false,
  };

  assert_eq!(released(&dir, Some(FAR), Some(OWN)).place(&ask), None);
  assert!(!dir.join("bench").exists());
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
  let kept = fill_marks(&LANES.placed.kept, &[("variable", "CONSOLE_URL")]);

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
      to_free_seat: false,
    },
    &dir,
    "machine-2",
  );
  let bench = ReadyBench {
    path: dir.to_string_lossy().into_owned(),
    checkouts: Vec::new(),
  };
  // A stand-in `ssh` whose far side says its turn came and ends, whatever line it is handed.
  install(
    &format!("#!/bin/sh\necho \"{}\" >&2\nexit 4\n", LANES.placed.turn_came),
    &dir.join("ssh"),
  );

  let wide = lock_path(&env, Lane::Wide);

  assert_eq!(read_placed(&wide)[0].entry.since, None);
  assert_eq!(placement.run(&bench, &command, false, Some(&placed)), 4);
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
    ],
  );
  let pid: i64 = std::fs::read_to_string(&left).unwrap().trim().parse().unwrap();

  assert_eq!(done.code, 124, "{}\n{}", done.out, done.err);
  assert_eq!(done.out, "once\n");
  assert_eq!(done.err, format!("said\n{hung}\n"));
  assert!(!process_alive(pid));
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

// "A disk below the settings' disk_free_gigabytes ... sends a placed-disk-low event naming the machine, the
// floor and each path below it"; "a path whose free space does not read is said on a line"; "a dry run reads
// and says the free space and sends nothing"
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

  assert!(above[1].starts_with(&format!("  {target}: ")), "{above:?}");
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
