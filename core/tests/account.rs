//! The account's name, from the environment or, where it names none, from the password database.

use bmo_testbed_core::account::account_name;

fn id_name() -> String {
  let out = std::process::Command::new("id").arg("-un").output().unwrap();

  String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

#[test]
fn the_name_is_the_one_the_environment_gives() {
  match std::env::var("USER") {
    Ok(user) if !user.is_empty() => assert_eq!(account_name(), user),
    _ => assert_eq!(account_name(), id_name()),
  }
}

// Run again by the test below in a process of its own with no `USER`; passes over where it is not.
#[test]
fn where_the_environment_names_none_the_password_database_does_inner() {
  if std::env::var_os("ACCOUNT_TEST_INNER").is_none() {
    return;
  }

  assert!(std::env::var_os("USER").is_none());
  assert_eq!(account_name(), id_name());
}

#[test]
fn where_the_environment_names_none_the_password_database_does() {
  let ran = std::process::Command::new(std::env::current_exe().unwrap())
    .args(["--exact", "account::where_the_environment_names_none_the_password_database_does_inner"])
    .env_remove("USER")
    .env("ACCOUNT_TEST_INNER", "1")
    .output()
    .unwrap();

  assert!(ran.status.success(), "{}", String::from_utf8_lossy(&ran.stdout));
  assert!(String::from_utf8_lossy(&ran.stdout).contains("1 passed"));
}
