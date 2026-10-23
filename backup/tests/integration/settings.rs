//! The settings (docs/backup.md, 「The settings」): what a file leaves out takes the policy's default, a
//! key the testbed does not know refuses the run, and a plan names what it dumps.

use std::path::Path;

use bmo_testbed_backup::policy::BACKUP;
use bmo_testbed_backup::settings::{Plan, Set, Settings, expanded, settings_path};

use crate::stand::Ground;

#[test]
fn what_the_file_leaves_out_takes_the_policys_default() {
  let ground = Ground::new();
  let plan = ground.plan(&ground.settings("folders = [\"brain\"]\ndatabases = [\"app\"]"));
  let defaults = &BACKUP.defaults;

  assert_eq!(plan.hourly_slots, defaults.hourly_slots);
  assert_eq!(plan.daily_days, defaults.daily_days);
  assert_eq!(plan.releases_kept, defaults.releases_kept);
  assert_eq!(plan.large_file_bytes, defaults.large_file_bytes);
  assert_eq!(plan.prefix, defaults.prefix);
  assert_eq!(plan.folders, [ground.path("root/brain")]);
  assert_eq!(plan.destinations.len(), 2);
  assert!(
    plan
      .destinations
      .iter()
      .all(|one| [Set::Hourly, Set::Daily, Set::Media, Set::Releases].iter().all(|set| one.takes(*set)))
  );
}

#[test]
fn a_key_the_testbed_does_not_know_refuses_the_run() {
  let ground = Ground::new();
  let text = ground.settings("folders = []\ndatabases = []\nhourly = 3");

  assert!(toml::from_str::<Settings>(&text).is_err());

  let path = ground.path("backup.toml");

  std::fs::write(&path, text).unwrap();

  let refused = Plan::read(&path, ground.dir.path()).err().unwrap();

  assert!(refused.contains("do not read"), "{refused}");
}

#[test]
fn settings_without_a_destination_are_refused() {
  let settings: Settings = toml::from_str("root = \"/r\"\nfolders = []\ndatabases = []").unwrap();
  let refused = Plan::of(settings, Path::new("/home")).err().unwrap();

  assert!(refused.contains("no [[destination]]"), "{refused}");
}

#[test]
fn a_destination_missing_what_its_kind_needs_is_refused() {
  let text =
    "root = \"/r\"\nfolders = []\ndatabases = []\n[[destination]]\nname = \"one\"\nkind = \"s3\"\n";
  let refused = Plan::of(toml::from_str(text).unwrap(), Path::new("/home")).err().unwrap();

  assert!(refused.contains("bucket"), "{refused}");
}

#[test]
fn a_tilde_stands_for_the_home_directory() {
  assert_eq!(
    expanded("~/.local/state", Path::new("/home/user")),
    Path::new("/home/user/.local/state")
  );
  assert_eq!(expanded("~", Path::new("/home/user")), Path::new("/home/user"));
  assert_eq!(expanded("/srv/store", Path::new("/home/user")), Path::new("/srv/store"));
  assert_eq!(
    settings_path(Path::new("/data")),
    Path::new("/data/env").join(&BACKUP.names.settings_file)
  );
}

#[test]
fn a_plan_dumps_what_its_globs_name_and_never_the_drills_own() {
  let ground = Ground::new();
  let plan = ground.plan(&ground.settings(
    "folders = []\ndatabases = [\"app\", \"app_*\"]\nexclude_databases = [\"app_test*\", \"app_*[0-9]\"]",
  ));

  assert!(plan.dumps("app"));
  assert!(plan.dumps("app_orders"));
  assert!(!plan.dumps("app_test"));
  assert!(!plan.dumps("app_copy_1234"));
  assert!(!plan.dumps("other"));
  assert!(!plan.dumps(&format!("{}app", BACKUP.names.drill_database)));
}

#[test]
fn a_database_with_an_address_file_is_dumped_through_what_the_file_holds() {
  let ground = Ground::new();
  let file = ground.path("app-address");

  std::fs::write(&file, "postgres:///app?host=/run/db&user=app_role\n").unwrap();

  let plan = ground.plan(&ground.settings(&format!(
    "folders = []\ndatabases = [\"app\", \"other\"]\nserver = \"postgres:///postgres?host=/run/db\"\n\
     address_files = {{ app = \"{}\" }}",
    file.display()
  )));

  assert_eq!(plan.address_of("app").unwrap(), "postgres:///app?host=/run/db&user=app_role");
  assert_eq!(plan.address_of("other").unwrap(), "postgres:///other?host=/run/db");
  assert!(
    ground
      .plan(&ground.settings("folders = []\ndatabases = []\naddress_files = { app = \"/nowhere\" }"))
      .address_of("app")
      .is_err()
  );
}
