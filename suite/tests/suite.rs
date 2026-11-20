//! The suite's own terms (docs/suite.md): no guard stands over a unit or a run, every unit runs in a sweep a width at a time with the units that must run alone in lanes of
//! one, the run's lines open with the mark and its last line says on its own how it came out, and the fence
//! holds it apart.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::Duration;

use bmo_testbed_core::declaration::Test;
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::suite::SUITE;
use bmo_testbed_core::turn::machine_width;
use bmo_testbed_suite::fence::{
  ClearSeams, FenceLimits, FenceSeams, SUITE_SLICE, clear_fence, fence_call, fence_limits, fence_suite,
  fence_unit, fence_units, fenced_build_jobs, other_fence_listed,
};
use bmo_testbed_suite::run::{
  SuiteWords, SweepCount, ask_of, file_groups, runner_of, script_runner, sweep_ending, unit_account,
  verdict_line,
};
use bmo_testbed_suite::sweep::{SweepPlan, largest_first, sweep};
use bmo_testbed_suite::unit::UnitRun;

fn words(given: &[&str]) -> Vec<String> {
  given.iter().map(|word| (*word).to_owned()).collect()
}

mod guards {
  use super::*;

  // "a unit is never cut for its length as a whole": a declaration that leaves the old keys out names no span
  // a unit or a run must end within.
  #[test]
  fn a_declaration_names_no_span_a_unit_or_a_run_must_end_within() {
    let test = Test::default();

    assert_eq!((test.file_guard_seconds, test.guard_seconds), (None, None));
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
    let command = group.language.test_command(&group.files, &[]).unwrap();
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
    assert!(!passes(root, &group.language.test_command(&file, &[]).unwrap()));
  }
}

mod sweeping {
  use super::*;

  fn no_lane<T>(_: &T) -> Option<String> {
    None
  }

  fn after_none<T>(_: &T) -> Option<usize> {
    None
  }

  fn plan<'a, T>(width: usize, lane: &'a dyn Fn(&T) -> Option<String>) -> SweepPlan<'a, T> {
    SweepPlan {
      width,
      lane,
      after: &after_none,
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

    assert_eq!(done, waits);
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
    assert_eq!(sweep(&[1, 2], &plan(8, &no_lane), &|item: &i32| item * 2), vec![2, 4]);

    let none: [i32; 0] = [];
    let done = sweep(&none, &plan(8, &no_lane), &|_: &i32| 1);

    assert!(done.is_empty());
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
    // The first three items hold each other until all three have started, however late a place's thread is
    // scheduled: only a lane that starts with the sweep can be one of them, since the rest have two places
    // and both are held. The wait is bounded, so a sweep that holds its lane back fails rather than hangs.
    let entered = Mutex::new(0);
    let one_entered = Condvar::new();

    sweep(&items, &plan(3, &kind), &|item: &&str| {
      order.lock().unwrap().push(*item);
      most.fetch_max(all.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
      running.enter(kind(item));

      let mut so_far = entered.lock().unwrap();

      *so_far += 1;
      one_entered.notify_all();
      drop(
        one_entered
          .wait_timeout_while(so_far, Duration::from_secs(5), |so_far| *so_far < 3)
          .unwrap(),
      );

      running.leave(kind(item));
      all.fetch_sub(1, Ordering::SeqCst);
    });

    assert_eq!(running.most(Some("browser")), 1);
    assert_eq!(most.load(Ordering::SeqCst), 3);
    assert!(order.lock().unwrap()[..3].contains(&"b1"));
  }

  // "runs each lane from the start beside the others, one at a time within it": the crates and projects
  // never hold the browser files queued behind them back from starting.
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
    let entered = AtomicUsize::new(0);
    let done = sweep(&["b", "a"], &plan(1, &kind), &|item: &&str| {
      most.fetch_max(all.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
      entered.fetch_add(1, Ordering::SeqCst);

      // Each waits for the other to begin, however loaded the machine, and a sweep that runs them one after
      // the other lets the first go after the bound, having seen one at a time.
      let bound = std::time::Instant::now() + Duration::from_secs(5);

      while entered.load(Ordering::SeqCst) < 2 && std::time::Instant::now() < bound {
        std::thread::sleep(Duration::from_millis(1));
      }

      all.fetch_sub(1, Ordering::SeqCst);
      *item
    });

    assert_eq!(done, vec!["b", "a"]);
    assert_eq!(most.load(Ordering::SeqCst), 2);
  }

  // "every unit is started and run to its end": however long the units take, none is left unstarted.
  #[test]
  fn every_item_is_started_and_run_to_its_end() {
    let starts = AtomicUsize::new(0);
    let done = sweep(&[1, 2, 3, 4, 5, 6], &plan(2, &no_lane), &|item: &i32| {
      starts.fetch_add(1, Ordering::SeqCst);
      std::thread::sleep(Duration::from_millis(20));
      *item
    });

    assert_eq!(done, [1, 2, 3, 4, 5, 6]);
    assert_eq!(starts.load(Ordering::SeqCst), 6);
  }

  // "a project's browser file starts once its project's check has ended; while it waits, the lane takes the
  // next browser file that waits on nothing": the check is what installs the project's packages.
  #[test]
  fn an_item_starts_after_the_one_it_names_and_its_lane_takes_the_next_meanwhile() {
    let items = ["check", "own-browser", "declared-browser"];
    let kind = |item: &&str| match *item {
      "check" => Some("heavy".to_owned()),
      _ => Some("browser".to_owned()),
    };
    let after = |item: &&str| (*item == "own-browser").then_some(0);
    let check_ended = AtomicBool::new(false);
    let started = Mutex::new(Vec::new());
    let one_started = Condvar::new();
    let check_waited_out = AtomicBool::new(false);
    let saw_the_check_ended = AtomicBool::new(false);

    sweep(
      &items,
      &SweepPlan {
        width: 3,
        lane: &kind,
        after: &after,
      },
      &|item: &&str| {
        started.lock().unwrap().push(*item);
        one_started.notify_all();

        match *item {
          // The check goes on only once the lane has taken the browser file that waits on nothing; a lane
          // held by the file waiting on the check never takes it, and the wait runs out.
          "check" => {
            let (so_far, waited) = one_started
              .wait_timeout_while(started.lock().unwrap(), Duration::from_secs(5), |so_far| {
                !so_far.contains(&"declared-browser")
              })
              .unwrap();

            drop(so_far);
            check_waited_out.store(waited.timed_out(), Ordering::SeqCst);
            check_ended.store(true, Ordering::SeqCst);
          }
          "own-browser" => saw_the_check_ended.store(check_ended.load(Ordering::SeqCst), Ordering::SeqCst),
          _ => {}
        }
      },
    );

    assert!(!check_waited_out.load(Ordering::SeqCst));
    assert!(saw_the_check_ended.load(Ordering::SeqCst));
  }

  // "an item named in its own queue behind the one waiting on it is taken first", so a lane never waits on
  // itself.
  #[test]
  fn an_item_waiting_on_one_behind_it_in_its_own_lane_runs_second() {
    let items = ["waits", "first"];
    let kind = |_: &&str| Some("browser".to_owned());
    let after = |item: &&str| (*item == "waits").then_some(1);
    let order = Mutex::new(Vec::new());

    sweep(
      &items,
      &SweepPlan {
        width: 1,
        lane: &kind,
        after: &after,
      },
      &|item: &&str| order.lock().unwrap().push(*item),
    );

    assert_eq!(*order.lock().unwrap(), ["first", "waits"]);
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
    assert!(verdict_line(true, 66.0, "495 of 495 files passed").starts_with(&SUITE.lines.mark));
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

  // "one ended by a signal names the signal under its heading, and one that failed having said nothing names
  // the exit code it ended with"
  #[test]
  fn a_unit_says_how_its_process_ended_where_its_output_cannot() {
    let said = &SUITE.file;
    let ran = |code: i32, signal: Option<i32>, output: &str| UnitRun {
      code,
      signal,
      seconds: 0.0,
      output: output.to_owned(),
    };
    let ending =
      |run: &UnitRun| unit_account("a.test.ts", run).lines().last().unwrap_or_default().to_owned();

    assert_eq!(ending(&ran(137, Some(9), "")), fill_marks(&said.signalled, &[("signal", "9")]));
    assert_eq!(ending(&ran(137, Some(9), "partial")), fill_marks(&said.signalled, &[("signal", "9")]));
    assert_eq!(ending(&ran(1, None, "\n")), fill_marks(&said.silent, &[("code", "1")]));
    assert_eq!(ending(&ran(1, None, "1 fail\n")), "1 fail");
    assert_eq!(
      ending(&ran(0, None, "")),
      fill_marks(&said.heading, &[("mark", &said.ok), ("seconds", "0.0"), ("file", "a.test.ts")])
    );
  }
}

mod endings {
  use super::*;

  // Ten files, as many of them passed and failed as given.
  fn counted(passed: usize, failed: usize) -> SweepCount {
    SweepCount {
      files: 10,
      passed,
      failed,
      skipped: 0,
    }
  }

  // "passes only where every file passed"
  #[test]
  fn passes_only_where_every_file_passed() {
    let (ok, note, code) = sweep_ending(counted(10, 0));

    assert!(ok);
    assert_eq!(note, fill_marks(&SUITE.ending.all_passed, &[("files", "10")]));
    assert_eq!(code, 0);
  }

  // "is a failure wherever a file failed"
  #[test]
  fn is_a_failure_wherever_a_file_failed() {
    let said = &SUITE.ending;
    let note = [
      fill_marks(&said.passed, &[("passed", "9"), ("files", "10")]),
      fill_marks(&said.failed_count, &[("count", "1")]),
    ]
    .join(&said.joiner);

    assert_eq!(sweep_ending(counted(9, 1)), (false, note, 1));
  }

  // "a run that passed with tests skipped says so on its last line": the skip stands beside the files passed
  // and beside the files failed alike.
  #[test]
  fn names_the_tests_skipped_on_a_pass_and_a_failure_alike() {
    let said = &SUITE.ending;
    let skipped = fill_marks(&said.skipped_count, &[("count", "6")]);
    let passed = [fill_marks(&said.all_passed, &[("files", "10")]), skipped.clone()].join(&said.joiner);
    let failed = [
      fill_marks(&said.passed, &[("passed", "9"), ("files", "10")]),
      fill_marks(&said.failed_count, &[("count", "1")]),
      skipped,
    ]
    .join(&said.joiner);

    assert_eq!(
      sweep_ending(SweepCount {
        skipped: 6,
        ..counted(10, 0)
      }),
      (true, passed, 0)
    );
    assert_eq!(
      sweep_ending(SweepCount {
        skipped: 6,
        ..counted(9, 1)
      }),
      (false, failed, 1)
    );
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

  // "asks the manager for a scope around the running process, with its ceilings, in the suite's slice"
  #[test]
  fn asks_the_manager_for_a_scope_around_the_running_process() {
    let call = fence_call("bmo-suite-7.scope", 7, LIMITS);

    assert_eq!(call[..3], words(&["busctl", "--user", "call"]));
    assert!(call.contains(&"bmo-suite-7.scope".to_owned()));
    assert!(call.join(" ").contains(
      "7 PIDs au 1 7 MemoryMax t 8589934592 MemorySwapMax t 1073741824 TasksMax t 8192 OOMPolicy s continue \
       Slice s bmo-suite.slice"
    ));
    assert_eq!(call.last().map(String::as_str), Some("0"));
    assert!(Path::new(env!("CARGO_MANIFEST_DIR")).join("../units").join(SUITE_SLICE).is_file());
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

  // "restarts it, but only where ... no other run's fence stands": the manager's listing of the fences names
  // another only where a scope other than the run's own stands in it, with or without the mark of a failed unit.
  #[test]
  fn another_run_s_fence_is_found_in_the_manager_s_listing() {
    let own = fence_unit(4242);
    let other = fence_unit(77);
    let line = |unit: &str| format!("{unit} loaded active running Scope\n");

    assert!(fence_units().starts_with(own.split("4242").next().unwrap()));
    assert!(!other_fence_listed("", 4242));
    assert!(!other_fence_listed(&line(&own), 4242));
    assert!(other_fence_listed(&format!("{}\u{25cf} {}", line(&own), line(&other)), 4242));
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
