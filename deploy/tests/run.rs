//! What every part of a deployment stands on: a moment written for a name, the guarded removal
//! (docs/deploy.md, 「What it refuses」), and the transient unit a deploy run stands in (「Where a
//! deployment runs」).

use std::path::{Path, PathBuf};

use bmo_testbed_core::policy::deploy::DEPLOY;
use bmo_testbed_core::policy::fill_shown;
use bmo_testbed_deploy::remove::{git_tree_above, removal_fault, remove_tree};
use bmo_testbed_deploy::run::{run_quiet, shell_words, stamp_for_name, words};
use bmo_testbed_deploy::unit::{
  DeployUnitStart, deploy_unit_line, deploy_unit_name, deploy_unit_notice, first_chars, upload_unit_name,
};

#[test]
fn a_moment_is_written_for_a_name() {
  assert_eq!(stamp_for_name(1_791_670_849_983), "2026-10-10T22-20-49-983Z");
}

#[test]
fn a_declared_command_line_runs_through_the_shell_as_it_is_written() {
  let ran = run_quiet(&shell_words("printf 'one two' && printf ' three' >&2"), None);

  assert!(ran.ok);
  assert_eq!(ran.out, "one two three");
  assert!(!run_quiet(&words(["false"]), None).ok);
  assert!(!run_quiet(&words(["a-command-nobody-has"]), None).ok);
}

#[test]
fn nothing_inside_the_data_root_or_above_it_is_removed() {
  assert_eq!(
    removal_fault(Path::new("/srv/data"), Path::new("/srv/data/locks")),
    Some(fill_shown(
      &DEPLOY.remove.inside,
      &[("path", &"/srv/data/locks"), ("data", &"/srv/data")]
    ))
  );
  assert_eq!(
    removal_fault(Path::new("/srv/data"), Path::new("/srv")),
    Some(fill_shown(&DEPLOY.remove.holds, &[("path", &"/srv"), ("data", &"/srv/data")]))
  );
  assert!(removal_fault(Path::new("/srv/data"), Path::new("/srv/backups/x")).is_none());
}

#[test]
fn the_guarded_removal_takes_a_tree_away_and_refuses_one_holding_the_data_root() {
  let root = tempfile::tempdir().unwrap();
  let data = root.path().join("app/data");
  let other = root.path().join("other");

  std::fs::create_dir_all(&data).unwrap();
  std::fs::create_dir_all(other.join("deep")).unwrap();
  std::fs::write(other.join("deep/file"), "").unwrap();

  assert!(remove_tree(&root.path().join("app"), &data).is_err());
  assert!(data.exists());
  remove_tree(&other, &data).unwrap();
  assert!(!other.exists());
  // What is not there is gone already.
  remove_tree(&other, &data).unwrap();
}

#[test]
fn the_git_tree_a_path_stands_in_is_the_nearest_folder_holding_git() {
  let root = tempfile::tempdir().unwrap();

  std::fs::create_dir_all(root.path().join("repo/.git")).unwrap();
  std::fs::create_dir_all(root.path().join("repo/data/inner")).unwrap();

  assert_eq!(git_tree_above(&root.path().join("repo/data/inner")), Some(root.path().join("repo")));
}

// "the unit line puts the unit in its slice, hands over the marker, the PATH, the roots and the working tree,
// then the command"
#[test]
fn the_unit_line_hands_over_the_slice_the_marker_the_path_the_roots_and_the_working_tree_then_the_command()
{
  let line = deploy_unit_line(
    &DeployUnitStart {
      unit: "bmo-deploy-9".to_owned(),
      slice: "work.slice".to_owned(),
      cwd: PathBuf::from("/tree"),
      command: words(["/tree/bin/tool", "deploy", "--then", "why"]),
      env: vec![("BMO_DATA_DIR".to_owned(), "/data".to_owned())],
    },
    "/bin:/usr/bin",
  );

  assert_eq!(&line[..5], ["systemd-run", "--user", "--quiet", "--collect", "--unit=bmo-deploy-9"]);
  assert!(line.contains(&"--slice=work.slice".to_owned()));
  assert!(line.contains(&"--setenv=BMO_DEPLOY_UNIT=bmo-deploy-9".to_owned()));
  assert!(line.contains(&"--setenv=PATH=/bin:/usr/bin".to_owned()));
  assert!(line.contains(&"--setenv=BMO_DATA_DIR=/data".to_owned()));
  assert!(line.contains(&"--working-directory=/tree".to_owned()));
  assert_eq!(&line[line.len() - 4..], ["/tree/bin/tool", "deploy", "--then", "why"]);
}

#[test]
fn a_unit_is_named_for_what_it_runs_and_its_notice_names_its_journal() {
  assert_eq!(deploy_unit_name(5), "bmo-deploy-5");
  assert_eq!(upload_unit_name(5), "bmo-backup-upload-5");
  assert_eq!(
    deploy_unit_notice("bmo-deploy-5"),
    fill_shown(&DEPLOY.unit.notice, &[("unit", &"bmo-deploy-5")])
  );
  assert!(deploy_unit_notice("bmo-deploy-5").contains("journalctl --user -u bmo-deploy-5 --no-pager"));
  assert_eq!(first_chars("abcdef", 3), "abc");
}
