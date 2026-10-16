//! The testbed's `run` and `status` verbs (docs/boundary.md, 「The verbs」): the words they read, and the
//! words they refuse, with the code they end on and nothing placed or run.

use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::lanes::LANES;
use bmo_testbed_core::verb::{Asked, REFUSED_EXIT_CODE, Refused, RunWords, VerbGround, main, read_words};

fn words(line: &[&str]) -> Vec<String> {
  line.iter().map(|word| (*word).to_owned()).collect()
}

// The verb run over the words where nothing may be placed, its code and whether it went to place a run.
fn refused_code(line: &[&str]) -> (u8, bool) {
  let dir = tempfile::tempdir().unwrap();
  let mut placed = false;
  let code = main(
    &words(line),
    &mut VerbGround {
      verb: "bmo-testbed",
      data_root: dir.path().to_path_buf(),
      close_line: None,
      place: &mut |_| {
        placed = true;
        Some(0)
      },
    },
  );

  (code, placed)
}

// "reads its own words before the command and the command after them"
#[test]
fn the_verb_reads_its_own_words_before_the_command_and_the_command_after_them() {
  assert_eq!(
    read_words(&words(&["--narrow", "--guard", "60", "--adapter", "cargo", "--", "cargo", "test"])),
    Ok(Asked::Run(RunWords {
      command: words(&["cargo", "test"]),
      narrow: true,
      adapter: Some("cargo".to_owned()),
      guard_seconds: Some(60.0),
      wait_seconds: None,
    }))
  );
  assert_eq!(
    read_words(&words(&["cargo", "test", "--", "--nocapture"])),
    Ok(Asked::Run(RunWords {
      command: words(&["cargo", "test", "--", "--nocapture"]),
      ..RunWords::default()
    }))
  );
  assert_eq!(read_words(&words(&["status", "--json"])), Ok(Asked::Status { json: true }));
  assert_eq!(read_words(&words(&["--help"])), Ok(Asked::Usage));
}

// "refuses a line with no command, or with a flag it does not read"
#[test]
fn a_line_with_no_command_or_a_flag_it_does_not_read_is_refused() {
  let said = &LANES.verb;

  assert_eq!(
    read_words(&[]),
    Err(Refused {
      said: said.no_command.clone(),
      hint: true
    })
  );
  assert_eq!(
    read_words(&words(&["--fast", "--", "true"])),
    Err(Refused {
      said: fill_marks(&said.flag_as_command, &[("word", "--fast")]),
      hint: true
    })
  );
  assert_eq!(
    read_words(&words(&["--wait", "0", "--", "true"])),
    Err(Refused {
      said: fill_marks(&said.seconds, &[("flag", "--wait")]),
      hint: false
    })
  );

  for line in [&[][..], &["--fast", "--", "true"], &["--adapter", "cobol", "--", "true"]] {
    assert_eq!(refused_code(line), (REFUSED_EXIT_CODE, false), "{line:?}");
  }
}

// The usage's defaults are the policy's own numbers, each a mark the verb fills.
#[test]
fn the_usage_names_the_verb_and_the_policys_numbers_by_their_marks() {
  for mark in ["{verb}", "{guard}", "{job_wait}", "{unit_wait}", "{kept}"] {
    assert!(LANES.verb.usage.contains(mark), "{mark}");
  }
}
