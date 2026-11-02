//! The event hook (docs/boundary.md, 「Events」): the hook the environment names over the declaration's,
//! handed the event's name and its payload, and nothing a failing hook does reaching the verb.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use bmo_testbed_core::declaration::Repository;
use bmo_testbed_core::events::{EVENT_HOOK_VARIABLE, Event, Hook};
use bmo_testbed_core::turn::Env;
use serde_json::{Value, json};

fn event() -> Event<'static> {
  Event {
    name: "suite-flaky",
    commit: Some("abc123".to_owned()),
    summary: "Two tests passed only the second time.".to_owned(),
    detail: json!({ "tests": [{ "test": "a > b", "unit": "a.test.ts" }], "units": [], "log": "" }),
  }
}

fn hook(command: &str) -> Hook {
  Hook {
    command: Some(command.to_owned()),
    repository: Some("sample".to_owned()),
    root: Some(PathBuf::from("/checkout")),
  }
}

// "the hook is handed the event's name as its first argument and the payload on its standard input"
#[test]
fn the_hook_is_handed_the_events_name_and_its_payload() {
  let dir = tempfile::tempdir().unwrap();
  let named = dir.path().join("named");
  let read = dir.path().join("read");

  hook(&format!("f() {{ echo \"$1\" > {}; cat > {}; }}; f", named.display(), read.display()))
    .send(&event());

  let payload: Value = serde_json::from_str(&std::fs::read_to_string(&read).unwrap()).unwrap();

  assert_eq!(std::fs::read_to_string(&named).unwrap(), "suite-flaky\n");
  assert_eq!(payload["version"], 1);
  assert_eq!(payload["event"], "suite-flaky");
  assert_eq!(payload["repository"], "sample");
  assert_eq!(payload["root"], "/checkout");
  assert_eq!(payload["commit"], "abc123");
  assert_eq!(payload["detail"]["tests"][0]["unit"], "a.test.ts");
  assert!(chrono::DateTime::parse_from_rfc3339(payload["at"].as_str().unwrap()).is_ok());
}

// "a hook that fails is said and goes no further"
#[test]
fn a_hook_that_fails_changes_nothing() {
  hook("exit 3").send(&event());
  hook("/no/such/hook-program").send(&event());
}

// "the hook is waited for until it ends": one that takes its time is never stopped
#[test]
fn a_hook_that_takes_its_time_is_waited_for_until_it_ends() {
  let dir = tempfile::tempdir().unwrap();
  let ended = dir.path().join("ended");
  let started = Instant::now();

  hook(&format!("sleep 1; touch {}", ended.display())).send(&event());

  assert!(ended.exists());
  assert!(started.elapsed() >= Duration::from_secs(1));
}

// "the environment's hook stands over the declaration's, and a verb outside a repository has it alone"
#[test]
fn the_environments_hook_stands_over_the_declarations() {
  let dir = tempfile::tempdir().unwrap();

  std::fs::write(
    dir.path().join("testbed.toml"),
    "testbed = 0\n\n[events]\nhook = \"declared\"\ntimeout_seconds = 7\n",
  )
  .unwrap();

  let repo = Repository::read(dir.path()).unwrap();
  let named = Env::from([(EVENT_HOOK_VARIABLE.to_owned(), "named".to_owned())]);

  assert_eq!(Hook::of(Some(&repo), &Env::new()).command.as_deref(), Some("declared"));
  assert_eq!(Hook::of(Some(&repo), &named).command.as_deref(), Some("named"));
  assert_eq!(Hook::of(None, &named).command.as_deref(), Some("named"));
  assert_eq!(Hook::of(None, &Env::new()).command, None);
  assert_eq!(Hook::of(None, &Env::new()).repository, None);
}
