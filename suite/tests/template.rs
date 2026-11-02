//! The suite's template database (docs/suite.md, 「The test database」): only a plain name is written into a
//! statement, a sweep takes only what no run still stands on, and a template is made once by the declared
//! migrate command, with its functions and its note, and dropped with its pool.

use bmo_testbed_core::policy::suite::SUITE;
use bmo_testbed_suite::template::{
  connect, drop_set, execute, is_database_name, is_made_note, make_once, orphaned, pool_held, standing,
  superseded,
};
use bmo_testbed_suite::test_server::{pool_prefix, test_server_up};
use sqlx::Connection as _;

fn words(given: &[&str]) -> Vec<String> {
  given.iter().map(|word| (*word).to_owned()).collect()
}

#[test]
fn only_a_plain_lower_case_name_is_one_the_verb_writes() {
  for name in ["bmo_test_tmpl_ab12", "x", "_x9"] {
    assert!(is_database_name(name), "{name}");
  }

  for name in ["Upper", "9lead", "quote\"d", "a b", ""] {
    assert!(!is_database_name(name), "{name}");
  }
}

#[test]
fn a_sweep_takes_only_a_template_superseded_a_day_ago_whose_pool_nobody_holds() {
  let kept = SUITE.template.superseded_kept_seconds;
  let now = 10 * kept;
  let made = |ago: u64| Some(format!("made {}", now - ago));
  let templates = vec![
    ("bmo_test_tmpl_keep".to_owned(), made(5 * kept)),
    ("bmo_test_tmpl_old".to_owned(), made(kept + 1)),
    ("bmo_test_tmpl_fresh".to_owned(), made(kept)),
    ("bmo_test_tmpl_held".to_owned(), made(5 * kept)),
    ("bmo_test_tmpl_unnoted".to_owned(), None),
    ("bmo_test_tmpl_odd".to_owned(), Some("made yesterday".to_owned())),
  ];
  let held = |prefix: &str| prefix == "bmo_test_held";

  assert_eq!(superseded(&templates, "bmo_test_tmpl_keep", now, held), vec!["bmo_test_tmpl_old"]);
}

#[test]
fn a_sweep_takes_a_scratch_database_once_the_process_named_in_it_is_gone() {
  let databases = words(&[
    "bmo_test_1760000000_41_sample",
    "bmo_test_tmpl_1760000000_42_x",
    "bmo_test_1760000000_43_living",
    "bmo_test_17600_44_short",
    "bmo_test_ab12_s1",
  ]);

  assert_eq!(
    orphaned(&databases, |pid| pid == 43),
    vec!["bmo_test_1760000000_41_sample", "bmo_test_tmpl_1760000000_42_x"]
  );
}

#[test]
fn a_pool_is_held_by_a_claim_whose_process_lives() {
  let dir = tempfile::tempdir().unwrap();
  let claim = |file: &str, pid: &str| std::fs::write(dir.path().join(file), pid).unwrap();

  claim("bmo_test_ab12_s1.claim", "41\n");
  claim("bmo_test_cd34_s2.claim", "42");
  claim("bmo_test_ef56_s1.other", "42");
  claim("bmo_test_gh78_s1.claim", "not a pid");

  let alive = |pid: i64| pid == 42;

  assert!(!pool_held(dir.path(), "bmo_test_ab12", alive));
  assert!(pool_held(dir.path(), "bmo_test_cd34", alive));
  assert!(!pool_held(dir.path(), "bmo_test_ef56", alive));
  assert!(!pool_held(dir.path(), "bmo_test_gh78", alive));
}

// On the suite's test server: a template of the test's own made once by a migrate command that writes one
// table where the variable names it, with its functions and its note, found by a second make, and dropped
// with a clone of its pool.
#[tokio::test]
async fn a_template_is_made_once_by_the_migrate_command_and_dropped_with_its_pool() {
  test_server_up().unwrap();

  let dir = tempfile::tempdir().unwrap();
  let name = format!("bmo_test_tmpl_verb{}", std::process::id());
  let clone = format!("{}_s1", pool_prefix(&name));
  let migrate = "psql -q -X -v ON_ERROR_STOP=1 -c 'CREATE TABLE notes (id bigint GENERATED ALWAYS AS IDENTITY)' \"$BMO_TEST_TEMPLATE_URL\"";
  let mut server = connect("postgres").await.unwrap();

  drop_set(&mut server, &name).await.unwrap();
  make_once(&mut server, &name, migrate, dir.path()).await.unwrap();
  assert!(standing(&mut server, &name).await.unwrap());
  assert!(!standing(&mut server, &format!("{name}_making")).await.unwrap());

  let note: Option<String> =
    sqlx::query_scalar("SELECT shobj_description(oid, 'pg_database') FROM pg_database WHERE datname = $1")
      .bind(&name)
      .fetch_one(&mut server)
      .await
      .unwrap();

  assert!(note.as_deref().is_some_and(is_made_note), "{note:?}");

  let mut template = connect(&name).await.unwrap();
  let made: Vec<String> = sqlx::query_scalar(
    "SELECT proname::text FROM pg_proc WHERE proname IN ('bmo_test_reset', 'bmo_test_count_on') UNION ALL SELECT tablename::text FROM pg_tables WHERE schemaname = 'public' ORDER BY 1",
  )
  .fetch_all(&mut template)
  .await
  .unwrap();

  template.close().await.unwrap();
  assert_eq!(made, ["bmo_test_count_on", "bmo_test_reset", "notes"]);

  make_once(&mut server, &name, "exit 9", dir.path()).await.unwrap();
  execute(&mut server, &format!("CREATE DATABASE \"{clone}\" TEMPLATE \"{name}\""))
    .await
    .unwrap();
  drop_set(&mut server, &name).await.unwrap();
  assert!(!standing(&mut server, &name).await.unwrap());
  assert!(!standing(&mut server, &clone).await.unwrap());
  server.close().await.unwrap();
}

// "a migrate command that fails leaves nothing standing, and says how it ended"
#[tokio::test]
async fn a_migrate_command_that_fails_leaves_no_template() {
  test_server_up().unwrap();

  let dir = tempfile::tempdir().unwrap();
  let name = format!("bmo_test_tmpl_fails{}", std::process::id());
  let mut server = connect("postgres").await.unwrap();
  let why = make_once(&mut server, &name, "echo broken >&2; exit 4", dir.path()).await.unwrap_err();

  assert!(why.contains("4") && why.contains("broken"), "{why}");
  assert!(!standing(&mut server, &name).await.unwrap());
  assert!(!standing(&mut server, &format!("{name}_making")).await.unwrap());
  server.close().await.unwrap();
}
