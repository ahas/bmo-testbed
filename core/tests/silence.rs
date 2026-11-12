//! A held command that falls silent is ended as hung (docs/testbed.md, 「A holder whose command falls silent
//! lets go」): each test watches a command of its own with a silence of a fraction of a second, so it ends in
//! seconds.

use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::lanes::LANES;
use bmo_testbed_core::queue::process_alive;
use bmo_testbed_core::silence::{HUNG_EXIT_CODE, Output, Silence, Watch};

fn short() -> Silence {
  Silence {
    limit: Duration::from_millis(400),
    grace: Duration::from_millis(300),
    look: Duration::from_millis(50),
  }
}

// A shell line started with its output into `out`, heading a process group of its own where `grouped`, handed
// the watch's variable.
fn started(line: &str, out: &Path, watch: &Watch, grouped: bool) -> Child {
  let file = std::fs::File::create(out).unwrap();
  let mut command = Command::new("sh");

  command
    .args(["-c", line])
    .envs(watch.variable())
    .stdin(Stdio::null())
    .stdout(file.try_clone().unwrap())
    .stderr(file);

  if grouped {
    command.process_group(0);
  }

  command.spawn().unwrap()
}

// The process a command wrote the number of into `file`.
fn written_pid(file: &Path) -> i64 {
  std::fs::read_to_string(file).unwrap().trim().parse().unwrap()
}

// "a command that writes nothing for the limit is ended as hung": it said one thing and went quiet; it ends with
// the hung code and the line naming it, its group and what it started gone with it, long before its sleep.
#[test]
fn a_command_that_writes_nothing_for_the_limit_is_ended_as_hung_with_everything_under_it() {
  let dir = tempfile::tempdir().unwrap();
  let out = dir.path().join("out.log");
  let left = dir.path().join("left");
  let watch = Watch::new(Output::Files(vec![out.clone()]), short());
  let started_at = Instant::now();
  let mut child =
    started(&format!("echo once; sleep 30 & echo $! > {}; wait", left.display()), &out, &watch, true);
  let waited = watch.wait(&mut child, "the stuck command");

  assert!(started_at.elapsed() < Duration::from_secs(10));
  assert_eq!(waited.code(), Some(i32::from(HUNG_EXIT_CODE)));
  assert_eq!(
    waited.hung.as_deref(),
    Some(
      fill_marks(
        &LANES.silence.hung,
        &[
          ("command", "the stuck command"),
          ("output", &std::fs::canonicalize(&out).unwrap().to_string_lossy()),
          ("seconds", "0.4"),
        ],
      )
      .as_str()
    )
  );
  assert!(!process_alive(written_pid(&left)));
}

// "every process under it" is ended where the command heads no process group of its own.
#[test]
fn a_hung_command_heading_no_group_of_its_own_is_ended_with_every_process_under_it() {
  let dir = tempfile::tempdir().unwrap();
  let out = dir.path().join("out.log");
  let left = dir.path().join("left");
  let watch = Watch::new(Output::Files(vec![out.clone()]), short());
  let mut child = started(
    &format!("echo once; sleep 30 & echo $! > {}; wait", left.display()),
    &out,
    &watch,
    false,
  );
  let waited = watch.wait(&mut child, "the stuck command");

  assert_eq!(waited.code(), Some(i32::from(HUNG_EXIT_CODE)));
  assert!(!process_alive(written_pid(&left)));
}

// "its length alone never cuts it": a command that keeps writing past the limit runs until it ends.
#[test]
fn a_command_that_keeps_writing_past_the_limit_runs_until_it_ends() {
  let dir = tempfile::tempdir().unwrap();
  let out = dir.path().join("out.log");
  let watch = Watch::new(Output::Files(vec![out.clone()]), short());
  let started_at = Instant::now();
  let mut child =
    started("for i in 1 2 3 4 5 6 7 8 9 10 11 12; do echo $i; sleep 0.1; done", &out, &watch, true);
  let waited = watch.wait(&mut child, "the busy command");

  assert!(started_at.elapsed() > short().limit);
  assert_eq!((waited.code(), waited.hung), (Some(0), None));
}

// "a command under it that is watched in turn speaks for it": a command writing nothing of its own but touching
// the file it was handed is not ended.
#[test]
fn a_touch_of_the_handed_file_counts_as_the_command_writing() {
  let dir = tempfile::tempdir().unwrap();
  let out = dir.path().join("out.log");
  let watch = Watch::new(Output::Files(vec![out.clone()]), short());
  let mut child = started(
    "for i in 1 2 3 4 5 6 7 8 9 10 11 12; do touch \"$BMO_TESTBED_HEARD\"; sleep 0.1; done",
    &out,
    &watch,
    true,
  );
  let waited = watch.wait(&mut child, "the quiet command");

  assert_eq!((waited.code(), waited.hung), (Some(0), None));
}

// "a command whose output is a terminal or a pipe is not watched": where any stream is not a regular file there
// is nothing to watch, and a command with nothing to watch is waited for until it ends.
#[test]
fn output_that_is_not_all_files_is_not_watched() {
  let dir = tempfile::tempdir().unwrap();
  let file = dir.path().join("out.log");

  std::fs::write(&file, "").unwrap();

  assert!(Output::files_only([file.clone(), file.clone()]).watched());
  assert!(!Output::files_only([file.clone(), PathBuf::from("/dev/null")]).watched());

  let watch = Watch::new(Output::files_only([PathBuf::from("/dev/null")]), short());
  let mut child = Command::new("sh").args(["-c", "sleep 1"]).stdout(Stdio::null()).spawn().unwrap();
  let waited = watch.wait(&mut child, "an unwatched command");

  assert!(watch.variable().is_none());
  assert_eq!((waited.code(), waited.hung), (Some(0), None));
}
