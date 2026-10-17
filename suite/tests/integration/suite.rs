//! The suite's own terms (docs/suite.md): the guards a declaration leaves at their defaults stand well above
//! the work, every unit runs in a sweep a width at a time with the units that must run alone in lanes of
//! one, the run's lines open with the mark and its last line says on its own how it came out, and the fence
//! holds it apart.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use bmo_testbed_core::declaration::Test;
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::suite::SUITE;
use bmo_testbed_core::turn::{NOT_RUN_EXIT_CODE, machine_width};
use bmo_testbed_suite::fence::{
  ClearSeams, FenceLimits, FenceSeams, clear_fence, fence_call, fence_limits, fence_suite, fence_unit,
  fenced_build_jobs,
};
use bmo_testbed_suite::run::{
  SuiteWords, SweepCount, ask_of, file_groups, guards_line, runner_of, script_runner, sweep_ending,
  verdict_line,
};
use bmo_testbed_suite::sweep::{SweepPlan, largest_first, sweep};

fn words(given: &[&str]) -> Vec<String> {
  given.iter().map(|word| (*word).to_owned()).collect()
}

mod guards {
  use super::*;

  // "thirty seconds per test, ninety per file and four minutes for the whole run", where a declaration
  // leaves them at their defaults.
  #[test]
  fn are_thirty_seconds_per_test_ninety_per_file_and_four_minutes_for_the_whole_run() {
    let test = Test::default();

    assert_eq!(test.test_timeout_seconds, 30);
    assert_eq!(test.file_guard_seconds, 90);
    assert_eq!(test.guard_seconds, 4 * 60);
  }

  // "stand well above the work": the whole run lets the slowest file run its whole guard twice over.
  #[test]
  fn stand_well_above_the_work() {
    let test = Test::default();

    assert!(test.guard_seconds >= 2 * test.file_guard_seconds);
    assert!(test.file_guard_seconds >= 3 * test.test_timeout_seconds);
  }

  // "is the machine's width for the one command holding it": a run holds the testbed's wide lane.
  #[test]
  fn the_sweep_is_the_machine_s_width_for_the_one_command_holding_it() {
    assert_eq!(machine_width(&HashMap::new(), 16), 16);
    assert_eq!(machine_width(&HashMap::from([("BMO_TEST_PARALLEL".to_owned(), "6".to_owned())]), 16), 6);
  }
}

mod asked {
  use super::*;

  // "a guard is the declared one where none was given, and the number where one was"
  #[test]
  fn a_guard_is_the_declared_one_unless_the_caller_names_one() {
    let test = Test::default();
    let declared = ask_of(&SuiteWords::default(), &test);
    let named = ask_of(
      &SuiteWords {
        guard: Some(30),
        file_guard: Some(10),
        turn_wait: Some(5),
        ..SuiteWords::default()
      },
      &test,
    );

    assert_eq!((declared.whole_guard, declared.file_guard, declared.turn_wait), (240, 90, None));
    assert_eq!((named.whole_guard, named.file_guard, named.turn_wait), (30, 10, Some(5.0)));
  }

  // "a word naming a path picks the units, and the runner is handed every other word but a width"
  #[test]
  fn a_path_picks_the_units_and_the_runner_is_handed_the_rest() {
    let ask = ask_of(
      &SuiteWords {
        words: words(&["--bail", "server/a.test.ts", "--parallel=4", "core/"]),
        ..SuiteWords::default()
      },
      &Test::default(),
    );

    assert_eq!(ask.picked, words(&["server/a.test.ts", "core/"]));
    assert_eq!(ask.extra, words(&["--bail"]));
  }

  fn declared(languages: &[&str]) -> Test {
    Test {
      languages: languages.iter().map(|one| (*one).to_owned()).collect(),
      ..Test::default()
    }
  }

  // "a file named as a declared language names its test files is a path, by its extension"
  #[test]
  fn a_file_named_as_a_declared_language_names_its_tests_is_a_path() {
    let given = SuiteWords {
      words: words(&["a.test.mjs", "b.ts", "-t", "name"]),
      ..SuiteWords::default()
    };

    assert_eq!(ask_of(&given, &declared(&["node"])).picked, words(&["a.test.mjs"]));
    assert_eq!(ask_of(&given, &declared(&["bun"])).picked, words(&["b.ts"]));
  }

  // "each file is run by the first declared language whose naming it carries, else the first that runs files"
  #[test]
  fn each_file_is_run_by_the_declared_language_whose_naming_it_carries() {
    let both = declared(&["cargo", "bun", "node"]);
    let name = |file: &str| runner_of(&both, file).map(|language| language.name());

    assert_eq!(name("server/a.test.ts"), Some("bun"));
    assert_eq!(name("tools/b.test.mjs"), Some("node"));
    assert_eq!(name("checks/c.spec.ts"), Some("bun"));
    assert_eq!(runner_of(&declared(&["cargo"]), "a.test.ts").map(|language| language.name()), None);
    assert_eq!(script_runner(&both).map(|language| language.name()), Some("bun"));
    assert_eq!(script_runner(&declared(&["node"])).map(|language| language.name()), Some("node"));
    assert!(script_runner(&declared(&["cargo"])).is_none());
  }
}

mod folders {
  use std::process::{Command, Stdio};

  use super::*;

  fn bun() -> Test {
    Test {
      languages: words(&["bun"]),
      ..Test::default()
    }
  }

  fn write(root: &Path, file: &str, text: &str) {
    let path = root.join(file);

    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
  }

  // What each group runs, read as plain values.
  fn groups(root: &Path, files: &[&str]) -> Vec<(String, PathBuf, Vec<String>)> {
    file_groups(root, &bun(), &words(files))
      .into_iter()
      .map(|group| (group.language.name().to_owned(), group.dir, group.files))
      .collect()
  }

  // "files under two folders holding their own configuration are two commands, and a file with none above it
  // runs from the root, each handed by its path from where it runs"
  #[test]
  fn files_are_one_group_per_folder_of_their_configuration_and_the_root_holds_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    write(root, "web/bunfig.toml", "");
    write(root, "api/tests/bunfig.toml", "");

    assert_eq!(
      groups(root, &["web/a.test.ts", "api/tests/b.test.ts", "lib/c.test.ts", "web/deep/d.test.ts"]),
      [
        ("bun".to_owned(), PathBuf::new(), words(&["lib/c.test.ts"])),
        ("bun".to_owned(), PathBuf::from("api/tests"), words(&["b.test.ts"])),
        ("bun".to_owned(), PathBuf::from("web"), words(&["a.test.ts", "deep/d.test.ts"])),
      ]
    );
  }

  // "a file whose folder's configuration preloads what its test reads passes only when run from that folder"
  #[test]
  fn a_file_under_its_own_configuration_passes_only_when_run_from_that_folder() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    write(root, "tests/ts/bunfig.toml", "[test]\npreload = [\"./preload.ts\"]\n");
    write(root, "tests/ts/preload.ts", "process.env.SUITE_PRELOADED = \"yes\";\n");
    write(
      root,
      "tests/ts/unit/reads.test.ts",
      "import { expect, test } from \"bun:test\";\n\ntest(\"reads what the preload set\", () => {\n  \
       expect(process.env.SUITE_PRELOADED).toBe(\"yes\");\n});\n",
    );

    let file = words(&["tests/ts/unit/reads.test.ts"]);
    let group = file_groups(root, &bun(), &file).remove(0);
    let command = group.language.test_command(&group.files, &[], None).unwrap();
    let passes = |at: &Path, command: &[String]| {
      Command::new(&command[0])
        .args(&command[1..])
        .current_dir(at)
        .env_remove("SUITE_PRELOADED")
        .stdin(Stdio::null())
        .output()
        .unwrap()
        .status
        .success()
    };

    assert_eq!(group.dir, Path::new("tests/ts"));
    assert!(passes(&root.join(&group.dir), &command));
    assert!(!passes(root, &group.language.test_command(&file, &[], None).unwrap()));
  }
}

mod sweeping {
  use super::*;

  fn no_lane<T>(_: &T) -> Option<String> {
    None
  }

  fn always() -> bool {
    true
  }

  fn plan<'a, T>(width: usize, lane: &'a dyn Fn(&T) -> Option<String>) -> SweepPlan<'a, T> {
    SweepPlan {
      width,
      lane,
      may_start: &always,
    }
  }

  // Counts what runs at once, by kind.
  #[derive(Default)]
  struct Running {
    now: Mutex<HashMap<Option<String>, usize>>,
    most: Mutex<HashMap<Option<String>, usize>>,
  }

  impl Running {
    fn enter(&self, kind: Option<String>) {
      let mut now = self.now.lock().unwrap();
      let count = now.entry(kind.clone()).or_default();

      *count += 1;

      let mut most = self.most.lock().unwrap();
      let best = most.entry(kind).or_default();

      *best = (*best).max(*count);
    }

    fn leave(&self, kind: Option<String>) {
      *self.now.lock().unwrap().get_mut(&kind).unwrap() -= 1;
    }

    fn most(&self, kind: Option<&str>) -> usize {
      self.most.lock().unwrap().get(&kind.map(str::to_owned)).copied().unwrap_or(0)
    }
  }

  // "gives the results back in the items' own order however they finished"
  #[test]
  fn gives_the_results_back_in_the_items_own_order() {
    let waits = [40u64, 5, 25, 1, 15];
    let done = sweep(&waits, &plan(2, &no_lane), &|wait: &u64| {
      std::thread::sleep(Duration::from_millis(*wait));
      *wait
    });

    assert_eq!(done.results, waits.iter().map(|wait| Some(*wait)).collect::<Vec<_>>());
    assert!(done.unstarted.is_empty());
  }

  // "never has more than the sweep's width running at once"
  #[test]
  fn never_has_more_than_the_width_running_at_once() {
    let running = AtomicUsize::new(0);
    let most = AtomicUsize::new(0);
    let items: Vec<usize> = (0..20).collect();

    sweep(&items, &plan(4, &no_lane), &|_: &usize| {
      let now = running.fetch_add(1, Ordering::SeqCst) + 1;

      most.fetch_max(now, Ordering::SeqCst);
      std::thread::sleep(Duration::from_millis(2));
      running.fetch_sub(1, Ordering::SeqCst);
    });

    assert_eq!(most.load(Ordering::SeqCst), 4);
  }

  // "runs everything even where the sweep is wider than the work, and nothing where there is none"
  #[test]
  fn runs_everything_where_the_sweep_is_wider_and_nothing_where_there_is_none() {
    assert_eq!(
      sweep(&[1, 2], &plan(8, &no_lane), &|item: &i32| item * 2).results,
      vec![Some(2), Some(4)]
    );

    let none: [i32; 0] = [];
    let done = sweep(&none, &plan(8, &no_lane), &|_: &i32| 1);

    assert!(done.results.is_empty() && done.unstarted.is_empty());
  }

  // "runs the items that must run alone one at a time, from the start, within the width": each browser file
  // drives a Chromium, and two side by side slow each other far more than they slow the rest.
  #[test]
  fn runs_the_items_that_must_run_alone_one_at_a_time_from_the_start() {
    let items = ["b1", "a1", "a2", "b2", "a3", "a4", "b3", "a5"];
    let kind = |item: &&str| item.starts_with('b').then(|| "browser".to_owned());
    let running = Running::default();
    let all = AtomicUsize::new(0);
    let most = AtomicUsize::new(0);
    let order = Mutex::new(Vec::new());

    sweep(&items, &plan(3, &kind), &|item: &&str| {
      order.lock().unwrap().push(*item);
      most.fetch_max(all.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
      running.enter(kind(item));
      std::thread::sleep(Duration::from_millis(5));
      running.leave(kind(item));
      all.fetch_sub(1, Ordering::SeqCst);
    });

    assert_eq!(running.most(Some("browser")), 1);
    assert_eq!(most.load(Ordering::SeqCst), 3);
    assert!(order.lock().unwrap()[..3].contains(&"b1"));
  }

  // "runs each lane from the start beside the others, one at a time within it": the crates and projects
  // take a whole run's guard between them, and the browser files queued behind them never started.
  #[test]
  fn runs_each_lane_from_the_start_beside_the_others() {
    let items = ["h1", "h2", "h3", "b1", "b2", "a1", "a2", "a3"];
    let kind = |item: &&str| match item.chars().next() {
      Some('h') => Some("heavy".to_owned()),
      Some('b') => Some("browser".to_owned()),
      _ => None,
    };
    let running = Running::default();
    let started = Mutex::new(Vec::new());
    let lane_started = std::sync::Condvar::new();
    let lanes_waited_out = AtomicUsize::new(0);

    sweep(&items, &plan(4, &kind), &|item: &&str| {
      let mut so_far = started.lock().unwrap();

      so_far.push(*item);
      lane_started.notify_all();
      // An item of no lane goes on only once both lanes have started beside it, so how soon a thread is
      // scheduled decides nothing; a lane queued behind these items never starts, and the wait runs out.
      if kind(item).is_none() {
        let (so_far, waited) = lane_started
          .wait_timeout_while(so_far, Duration::from_secs(5), |so_far| {
            !(so_far.contains(&"h1") && so_far.contains(&"b1"))
          })
          .unwrap();

        drop(so_far);
        if waited.timed_out() {
          lanes_waited_out.fetch_add(1, Ordering::SeqCst);
        }
      } else {
        drop(so_far);
      }
      running.enter(kind(item));
      std::thread::sleep(Duration::from_millis(5));
      running.leave(kind(item));
    });

    assert_eq!(running.most(Some("heavy")), 1);
    assert_eq!(running.most(Some("browser")), 1);
    assert_eq!(lanes_waited_out.load(Ordering::SeqCst), 0);
  }

  // "gives the rest a place of their own even where the width is one"
  #[test]
  fn gives_the_rest_a_place_of_their_own_even_where_the_width_is_one() {
    let kind = |item: &&str| (*item == "b").then(|| "browser".to_owned());
    let all = AtomicUsize::new(0);
    let most = AtomicUsize::new(0);
    let done = sweep(&["b", "a"], &plan(1, &kind), &|item: &&str| {
      most.fetch_max(all.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
      std::thread::sleep(Duration::from_millis(5));
      all.fetch_sub(1, Ordering::SeqCst);
      *item
    });

    assert_eq!(done.results, vec![Some("b"), Some("a")]);
    assert_eq!(most.load(Ordering::SeqCst), 2);
  }

  // "starts nothing once it may not, and hands back what never started": past the whole-run guard nothing
  // new starts, what is running finishes, and what was left is named.
  #[test]
  fn starts_nothing_once_it_may_not_and_hands_back_what_never_started() {
    let starts = AtomicUsize::new(0);
    let may_start = || starts.load(Ordering::SeqCst) < 3;
    let done = sweep(
      &[1, 2, 3, 4, 5, 6],
      &SweepPlan {
        width: 2,
        lane: &no_lane,
        may_start: &may_start,
      },
      &|item: &i32| {
        starts.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(2));
        *item
      },
    );

    assert_eq!(done.results[..3], [Some(1), Some(2), Some(3)]);
    assert_eq!(done.results[3..], [None, None, None]);
    assert_eq!(done.unstarted, vec![3, 4, 5]);
  }

  // "starts the largest first, since a large file is a long run"
  #[test]
  fn starts_the_largest_first() {
    let sizes =
      HashMap::from([("a.test.ts", 10u64), ("b.test.ts", 300), ("c.test.ts", 10), ("d.test.ts", 90)]);
    let files = words(&["a.test.ts", "b.test.ts", "c.test.ts", "d.test.ts"]);

    assert_eq!(
      largest_first(files, |file| sizes[file]),
      words(&["b.test.ts", "d.test.ts", "a.test.ts", "c.test.ts"])
    );
  }

  // "keeps files of one size in the order they came, and loses none"
  #[test]
  fn keeps_files_of_one_size_in_the_order_they_came() {
    assert_eq!(
      largest_first(words(&["c.test.ts", "a.test.ts"]), |_| 10),
      words(&["c.test.ts", "a.test.ts"])
    );
    assert!(largest_first(Vec::new(), |_| 0).is_empty());
  }
}

mod lines {
  use super::*;

  // "open with the mark, so a caller holding only the log can find them"
  #[test]
  fn open_with_the_mark() {
    assert!(guards_line(240, 90).starts_with(&SUITE.lines.mark));
    assert!(verdict_line(true, 66.0, "495 of 495 files passed").starts_with(&SUITE.lines.mark));
  }

  // "say at the head what the run is holding itself to"
  #[test]
  fn say_at_the_head_what_the_run_is_holding_itself_to() {
    assert_eq!(
      guards_line(240, 90),
      fill_marks(&SUITE.lines.guards, &[("mark", &SUITE.lines.mark), ("whole", "240"), ("file", "90")])
    );
  }

  // "say at the close how it came out and how long it took"
  #[test]
  fn say_at_the_close_how_it_came_out_and_how_long_it_took() {
    let said = &SUITE.ending;
    let verdict = |outcome: &str, seconds: &str, note: &str| {
      fill_marks(
        &said.verdict,
        &[("mark", &SUITE.lines.mark), ("outcome", outcome), ("seconds", seconds), ("note", note)],
      )
    };

    assert_eq!(verdict_line(true, 66.4, "n"), verdict(&said.ok, "66", "n"));
    assert_eq!(verdict_line(false, 334.0, "m"), verdict(&said.failed, "334", "m"));
  }
}

mod endings {
  use super::*;

  // Ten files, as many of them passed, failed, cut and never started as given.
  fn counted(passed: usize, failed: usize, cut: usize, unstarted: usize) -> SweepCount {
    SweepCount {
      files: 10,
      passed,
      failed,
      cut,
      unstarted,
    }
  }

  // "passes only where every file passed"
  #[test]
  fn passes_only_where_every_file_passed() {
    let (ok, note, code) = sweep_ending(counted(10, 0, 0, 0), 240, 90);

    assert!(ok);
    assert_eq!(note, fill_marks(&SUITE.ending.all_passed, &[("files", "10")]));
    assert_eq!(code, 0);
  }

  // "ends files that never started, and nothing else, as a run to make again"
  #[test]
  fn ends_files_that_never_started_as_a_run_to_make_again() {
    let said = &SUITE.ending;
    let note = [
      fill_marks(&said.passed, &[("passed", "7"), ("files", "10")]),
      fill_marks(&said.unstarted_count, &[("count", "3"), ("guard", "240")]),
    ]
    .join(&said.joiner);

    assert_eq!(
      sweep_ending(counted(7, 0, 0, 3), 240, 90),
      (false, fill_marks(&said.only_unstarted, &[("note", &note)]), NOT_RUN_EXIT_CODE)
    );
  }

  // "is a failure wherever a file failed or was cut, the unstarted still counted apart"
  #[test]
  fn is_a_failure_wherever_a_file_failed_or_was_cut() {
    let said = &SUITE.ending;
    let note = [
      fill_marks(&said.passed, &[("passed", "7"), ("files", "10")]),
      fill_marks(&said.failed_count, &[("count", "1")]),
      fill_marks(&said.cut_count, &[("count", "1"), ("guard", "90")]),
      fill_marks(&said.unstarted_count, &[("count", "1"), ("guard", "240")]),
    ]
    .join(&said.joiner);

    assert_eq!(sweep_ending(counted(7, 1, 1, 1), 240, 90), (false, note, 1));
  }
}

mod fence {
  use super::*;

  // A runtime directory holding the manager's private socket, which is all the refusal looks for.
  fn reachable_runtime() -> tempfile::TempDir {
    let runtime = tempfile::tempdir().unwrap();

    std::fs::create_dir_all(runtime.path().join("systemd")).unwrap();
    std::fs::write(runtime.path().join("systemd").join("private"), "").unwrap();
    runtime
  }

  // A manager that answers as told, and a run whose control group moves after `moves_after` looks.
  struct Manager {
    runtime: PathBuf,
    takes: bool,
    says: String,
    moves_after: usize,
    looks: usize,
    calls: Vec<Vec<String>>,
    offered: usize,
    paused: usize,
  }

  fn manager(runtime: &Path, moves_after: usize) -> Manager {
    Manager {
      runtime: runtime.to_path_buf(),
      takes: true,
      says: String::new(),
      moves_after,
      looks: 0,
      calls: Vec::new(),
      offered: 0,
      paused: 0,
    }
  }

  impl FenceSeams for Manager {
    fn pid(&self) -> u32 {
      4242
    }

    fn env(&self) -> Vec<(String, String)> {
      vec![("XDG_RUNTIME_DIR".to_owned(), self.runtime.to_string_lossy().into_owned())]
    }

    fn call(&mut self, command: &[String], _: &[(String, String)]) -> (bool, String) {
      self.calls.push(command.to_vec());
      (self.takes, self.says.clone())
    }

    fn cgroup(&mut self) -> String {
      self.looks += 1;

      if self.looks > self.moves_after {
        format!("0::/user.slice/app.slice/{}\n", fence_unit(4242))
      } else {
        "0::/user.slice/app.slice/terminal.service\n".to_owned()
      }
    }

    fn pause(&mut self) {
      self.paused += 1;
    }

    fn offer_first(&mut self) {
      self.offered += 1;
    }
  }

  const LIMITS: FenceLimits = FenceLimits {
    memory_gb: 8,
    swap_gb: 1,
    tasks: 8192,
  };

  // "holds a run to eight gigabytes unless the environment names another number"
  #[test]
  fn holds_a_run_to_eight_gigabytes_unless_named() {
    let numbers = &SUITE.numbers;

    // The swap and the tasks are the policy's, which the scope's call below spells out.
    assert_eq!((LIMITS.swap_gb, LIMITS.tasks), (numbers.fence_swap_gb, numbers.fence_tasks));
    assert_eq!(fence_limits(8, None), LIMITS);
    assert_eq!(fence_limits(8, Some("12")).memory_gb, 12);
    assert_eq!(fence_limits(8, Some("lots")).memory_gb, 8);
    assert_eq!(fence_limits(4, None).memory_gb, 4);
  }

  // "builds at as many Cargo jobs as its memory holds, within the width, keeping a caller's smaller number"
  #[test]
  fn builds_at_as_many_cargo_jobs_as_its_memory_holds() {
    // Eight gigabytes over the policy's build job, in tenths of a gigabyte.
    assert_eq!(fenced_build_jobs(LIMITS, 24, None) as u64, 8 * 10 / SUITE.numbers.build_job_tenths_gb);
    assert_eq!(fenced_build_jobs(LIMITS, 24, None), 5);
    assert_eq!(
      fenced_build_jobs(
        FenceLimits {
          memory_gb: 16,
          ..LIMITS
        },
        24,
        None
      ),
      10
    );
    assert_eq!(fenced_build_jobs(LIMITS, 3, None), 3);
    assert_eq!(
      fenced_build_jobs(
        FenceLimits {
          memory_gb: 1,
          ..LIMITS
        },
        24,
        None
      ),
      1
    );
    assert_eq!(fenced_build_jobs(LIMITS, 24, Some("2")), 2);
    assert_eq!(fenced_build_jobs(LIMITS, 24, Some("12")), 5);
    assert_eq!(fenced_build_jobs(LIMITS, 24, Some("many")), 5);
  }

  // "asks the manager for a scope around the running process, with its ceilings"
  #[test]
  fn asks_the_manager_for_a_scope_around_the_running_process() {
    let call = fence_call("bmo-suite-7.scope", 7, LIMITS);

    assert_eq!(call[..3], words(&["busctl", "--user", "call"]));
    assert!(call.contains(&"bmo-suite-7.scope".to_owned()));
    assert!(call.join(" ").contains(
      "6 PIDs au 1 7 MemoryMax t 8589934592 MemorySwapMax t 1073741824 TasksMax t 8192 OOMPolicy s continue"
    ));
    assert_eq!(call.last().map(String::as_str), Some("0"));
  }

  // "moves the run into its scope and names it"
  #[test]
  fn moves_the_run_into_its_scope_and_names_it() {
    let runtime = reachable_runtime();
    let mut run = manager(runtime.path(), 2);
    let line = fence_suite(LIMITS, &mut run);

    assert_eq!(
      line,
      fill_marks(
        &SUITE.lines.fence_held,
        &[
          ("mark", &SUITE.lines.mark),
          ("unit", "bmo-suite-4242.scope"),
          ("memory", "8"),
          ("swap", "1"),
          ("tasks", "8192"),
        ],
      )
    );
    assert_eq!((run.calls.len(), run.offered), (1, 1));
  }

  // "runs where it stands where no manager can be reached, and asks nothing"
  #[test]
  fn runs_where_it_stands_where_no_manager_can_be_reached() {
    let runtime = tempfile::tempdir().unwrap();
    let mut run = manager(runtime.path(), 0);
    let line = fence_suite(LIMITS, &mut run);
    let refusal = fill_marks(&SUITE.lines.no_socket, &[("runtime", &runtime.path().to_string_lossy())]);

    assert_eq!(
      line,
      fill_marks(&SUITE.lines.fence_none, &[("mark", &SUITE.lines.mark), ("why", &refusal)])
    );
    assert!(run.calls.is_empty());
    assert_eq!(run.offered, 1);
  }

  // "says what the manager answered where it refused the scope"
  #[test]
  fn says_what_the_manager_answered_where_it_refused() {
    let runtime = reachable_runtime();
    let mut run = manager(runtime.path(), 0);

    run.takes = false;
    run.says = "Access denied".to_owned();

    assert!(
      fence_suite(LIMITS, &mut run)
        .ends_with(&fill_marks(&SUITE.lines.fence_refused, &[("said", "Access denied")]))
    );
  }

  // "gives up on a move that never comes, after a bounded wait"
  #[test]
  fn gives_up_on_a_move_that_never_comes() {
    let runtime = reachable_runtime();
    let mut run = manager(runtime.path(), 1000);
    let line = fence_suite(LIMITS, &mut run);

    assert!(line.ends_with(&fill_marks(&SUITE.lines.fence_not_moved, &[("unit", "bmo-suite-4242.scope")])));
    assert_eq!(run.paused, SUITE.numbers.fence_move_tries as usize);
  }

  // A run standing in `group`, beside the processes `members` names; what it kills is kept.
  struct Kernel {
    group: String,
    members: Vec<u32>,
    killed: Vec<u32>,
    asked: Vec<String>,
  }

  fn kernel(group: &str, members: &[u32]) -> Kernel {
    Kernel {
      group: group.to_owned(),
      members: members.to_vec(),
      killed: Vec::new(),
      asked: Vec::new(),
    }
  }

  impl ClearSeams for Kernel {
    fn pid(&self) -> u32 {
      4242
    }

    fn cgroup(&mut self) -> String {
      format!("0::{}\n", self.group)
    }

    fn members(&mut self, group: &str) -> Vec<u32> {
      self.asked.push(group.to_owned());
      self.members.clone()
    }

    fn command(&mut self, pid: u32) -> String {
      if pid == 77 { String::new() } else { format!("bun stand-in.test.ts #{pid}") }
    }

    fn kill(&mut self, pid: u32) {
      self.killed.push(pid);
    }
  }

  // "kills what the files left in the run's own scope, and names it"
  #[test]
  fn kills_what_the_files_left_in_the_run_s_own_scope() {
    let scope = format!("/user.slice/app.slice/{}", fence_unit(4242));
    let mut run = kernel(&scope, &[4242, 51, 77]);
    let named = format!("51 bun stand-in.test.ts #51; 77 {}", SUITE.lines.gone);

    assert_eq!(
      clear_fence(&mut run),
      Some(fill_marks(
        &SUITE.lines.fence_killed,
        &[
          ("mark", &SUITE.lines.mark),
          ("count", "2"),
          ("processes", &SUITE.lines.processes),
          ("unit", "bmo-suite-4242.scope"),
          ("named", &named),
        ],
      ))
    );
    assert_eq!(run.asked, vec![scope]);
    assert_eq!(run.killed, vec![51, 77]);
  }

  // "says nothing where the run is alone in its scope"
  #[test]
  fn says_nothing_where_the_run_is_alone_in_its_scope() {
    let mut run = kernel(&format!("/user.slice/app.slice/{}", fence_unit(4242)), &[4242]);

    assert_eq!(clear_fence(&mut run), None);
    assert!(run.killed.is_empty());
  }

  // "kills nothing where the run stands in its caller's group"
  #[test]
  fn kills_nothing_where_the_run_stands_in_its_caller_s_group() {
    let mut run = kernel("/user.slice/app.slice/terminal.service", &[4242, 51]);

    assert_eq!(clear_fence(&mut run), None);
    assert!(run.asked.is_empty() && run.killed.is_empty());
  }
}

mod cargo {
  use bmo_testbed_suite::crates::checkout_target_dir;

  // Two checkouts of one directory name build apart, each under its own name and a hash of its whole path.
  #[test]
  fn two_checkouts_of_one_name_build_in_target_directories_of_their_own() {
    let one = checkout_target_dir(std::path::Path::new("/repos/a/sample"));
    let other = checkout_target_dir(std::path::Path::new("/repos/b/sample"));
    let named = |dir: &std::path::Path| dir.file_name().unwrap().to_string_lossy().into_owned();

    assert_ne!(one, other);
    assert_eq!(one.parent(), other.parent());
    assert!(named(&one).starts_with("sample-") && named(&one).len() == "sample-".len() + 12);
    assert_eq!(checkout_target_dir(std::path::Path::new("/repos/a/x/../sample")), one);
  }
}
