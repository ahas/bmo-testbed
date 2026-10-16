//! The part of a deployment with no service in it (docs/deploy.md): the refusal of a checkout, the
//! backup's name, the unit links and the marks units carry, and `bin/`'s mark.

use std::collections::HashMap;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use bmo_testbed_deploy::tree::{
  DeployRecord, UnitInstall, UnitLink, UnitMark, UnitRelink, backup_name, backup_stamp, base_name,
  binary_restarts, bot_unit, link_units, mark_built_from, marked_units, outside_relinks,
  read_deploy_record, relink_units, retired_units, set_aside_plain_units, timer_service, tree_fault,
  unit_link_plan, unit_links, unit_program, units_to_link, user_unit_dir_from, write_deploy_record,
};
use chrono::TimeZone as _;

fn names(paths: &[&str]) -> Vec<String> {
  paths.iter().map(|path| (*path).to_owned()).collect()
}

#[test]
fn the_record_is_written_whole_and_read_back_and_none_stands_for_no_record() {
  let dir = tempfile::tempdir().unwrap();
  let record = DeployRecord {
    commit: "b".to_owned(),
    subject: "s".to_owned(),
    at: "2026-09-21T00:00:00Z".to_owned(),
    backup: "x".to_owned(),
  };

  assert_eq!(read_deploy_record(dir.path()), None);
  write_deploy_record(&dir.path().join("deployed.json"), &record).unwrap();
  assert_eq!(read_deploy_record(dir.path()), Some(record));
}

#[test]
fn a_checkout_is_refused_for_this_runs_changes_for_no_commit_and_for_an_unpushed_one() {
  assert!(tree_fault(&names(&["a.ts"]), "c", true, "origin/main").unwrap().contains("a.ts"));
  assert_eq!(
    tree_fault(&[], "", true, "origin/main").as_deref(),
    Some("the working tree is on no commit at all")
  );
  assert_eq!(
    tree_fault(&[], &"c".repeat(40), false, "origin/main").as_deref(),
    Some("cccccccccccc is not on origin/main; an unpushed commit is not deployable")
  );
  assert_eq!(tree_fault(&[], &"c".repeat(40), true, "origin/main"), None);
}

#[test]
fn a_refusal_names_the_first_ten_changed_files_and_counts_the_rest() {
  let dirty: Vec<String> = (0..12).map(|one| format!("f{one}.rs")).collect();
  let said = tree_fault(&dirty, "c", true, "origin/main").unwrap();

  assert!(said.contains("f9.rs"));
  assert!(!said.contains("f10.rs"));
  assert!(said.ends_with(", and 2 more"));
}

fn local(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: u32) -> String {
  backup_stamp(chrono::Local.with_ymd_and_hms(year, month, day, hour, minute, second).unwrap())
}

// "is the declared name and the local time it was taken, down to the second"
#[test]
fn a_backup_is_named_for_the_local_time_it_was_taken_down_to_the_second() {
  assert_eq!(
    backup_name("app", &local(2026, 9, 7, 9, 4, 5), &[]).as_deref(),
    Some("app-20260907-090405.tar.zst")
  );
  assert_eq!(
    backup_name("site", &local(2026, 12, 31, 23, 59, 59), &[]).as_deref(),
    Some("site-20261231-235959.tar.zst")
  );
}

// "takes an ordinal when that second already has one"
#[test]
fn a_backup_takes_an_ordinal_when_that_second_already_has_one() {
  let stamp = local(2026, 9, 7, 9, 4, 5);
  let taken = names(&["app-20260907-090405.tar.zst"]);

  assert_eq!(backup_name("app", &stamp, &taken).as_deref(), Some("app-20260907-090405-02.tar.zst"));
  assert_eq!(
    backup_name(
      "app",
      &stamp,
      &names(&["app-20260907-090405.tar.zst", "app-20260907-090405-02.tar.zst"])
    )
    .as_deref(),
    Some("app-20260907-090405-03.tar.zst")
  );
}

// "is under the home's .config, or under XDG_CONFIG_HOME where it is set"
#[test]
fn the_user_units_are_under_the_homes_config_or_under_xdg_config_home() {
  let home = Path::new("/home/u");

  assert_eq!(user_unit_dir_from(None, home), Path::new("/home/u/.config/systemd/user"));
  assert_eq!(user_unit_dir_from(Some(""), home), Path::new("/home/u/.config/systemd/user"));
  assert_eq!(user_unit_dir_from(Some("/x/conf"), home), Path::new("/x/conf/systemd/user"));
}

fn link(name: &str, target: &str) -> UnitLink {
  UnitLink {
    name: name.to_owned(),
    target: PathBuf::from(target),
  }
}

// A plan over the deploy tree `/d/deploy`, against the commit's tracked files and the untracked ones.
fn plan(
  links: &[UnitLink],
  tracked: Option<&[&str]>,
  untracked: &[&str],
) -> (Vec<UnitRelink>, Vec<String>) {
  let tracked = names(tracked.unwrap_or(&["app.service", "site/app-web.service", "server/main.ts"]));

  unit_link_plan(links, Path::new("/d/deploy"), &tracked, &|path| untracked.contains(&path))
}

// "a link whose file the commit still has is left as it stands"
#[test]
fn a_link_whose_file_the_commit_still_has_is_left_as_it_stands() {
  assert_eq!(plan(&[link("app.service", "/d/deploy/app.service")], None, &[]), (vec![], vec![]));
}

// "a link the commit would leave dangling is relinked to the one file of the unit's own name"
#[test]
fn a_link_the_commit_would_leave_dangling_is_relinked_to_the_one_file_of_its_name() {
  assert_eq!(
    plan(&[link("app-web.service", "/d/deploy/web/app-web.service")], None, &[]),
    (
      vec![UnitRelink {
        name: "app-web.service".to_owned(),
        from: PathBuf::from("/d/deploy/web/app-web.service"),
        to: PathBuf::from("/d/deploy/site/app-web.service"),
      }],
      vec![]
    )
  );
}

// "a dangling link with no file of its name in the commit is a fault, named"
#[test]
fn a_dangling_link_with_no_file_of_its_name_is_a_fault_named() {
  let (relinks, faults) = plan(&[link("app-gone.service", "/d/deploy/web/app-gone.service")], None, &[]);

  assert!(relinks.is_empty());
  assert_eq!(faults.len(), 1);
  assert!(faults[0].contains("app-gone.service"));
  assert!(faults[0].contains("web/app-gone.service"));
}

// "and so is one with several files of its name to choose between"
#[test]
fn a_dangling_link_with_several_files_of_its_name_is_a_fault() {
  let (relinks, faults) = plan(
    &[link("app-web.service", "/d/deploy/web/app-web.service")],
    Some(&["site/app-web.service", "services/app-web.service"]),
    &[],
  );

  assert!(relinks.is_empty());
  assert!(faults[0].contains("2 files of that name"));
}

// "an untracked file standing there now is still there after the checkout"
#[test]
fn an_untracked_file_standing_there_now_is_still_there_after_the_checkout() {
  assert_eq!(
    plan(&[link("local.service", "/d/deploy/local.service")], None, &["local.service"]),
    (vec![], vec![])
  );
}

// "a link that points anywhere but the deploy tree is not the deployment's"
#[test]
fn a_link_that_points_anywhere_but_the_deploy_tree_is_not_the_deployments() {
  let links = [
    link("other.service", "/r/other/other.service"),
    link("neighbour.service", "/d/deploy-other/neighbour.service"),
  ];

  assert_eq!(plan(&links, None, &[]), (vec![], vec![]));
}

// A unit directory beside a deploy tree: a link to a file the tree has, one into it that dangles, a
// relative one that dangles, one pointing outside the tree, and a plain file.
fn installed_units() -> (tempfile::TempDir, PathBuf, PathBuf) {
  let root = tempfile::tempdir().unwrap();
  let units = root.path().join("units");
  let deploy = root.path().join("deploy");

  std::fs::create_dir_all(&units).unwrap();
  std::fs::create_dir_all(deploy.join("site")).unwrap();
  std::fs::write(deploy.join("app.service"), "[Unit]\n").unwrap();
  std::fs::write(deploy.join("site").join("app-web.service"), "[Unit]\n").unwrap();
  std::fs::write(units.join("plain.service"), "[Unit]\n").unwrap();
  symlink(deploy.join("app.service"), units.join("app.service")).unwrap();
  symlink(deploy.join("web").join("app-web.service"), units.join("app-web.service")).unwrap();
  symlink("../deploy/web/relative.service", units.join("relative.service")).unwrap();
  symlink(root.path().join("elsewhere").join("gone.service"), units.join("outside.service")).unwrap();

  (root, units, deploy)
}

// "reads only the links, a relative one against the directory it stands in"
#[test]
fn the_unit_links_are_only_the_links_a_relative_one_read_against_its_directory() {
  let (root, units, deploy) = installed_units();

  assert_eq!(
    unit_links(&units),
    [
      UnitLink {
        name: "app-web.service".to_owned(),
        target: deploy.join("web").join("app-web.service"),
      },
      UnitLink {
        name: "app.service".to_owned(),
        target: deploy.join("app.service"),
      },
      UnitLink {
        name: "outside.service".to_owned(),
        target: root.path().join("elsewhere").join("gone.service"),
      },
      UnitLink {
        name: "relative.service".to_owned(),
        target: deploy.join("web").join("relative.service"),
      },
    ]
  );
  assert!(unit_links(&root.path().join("absent")).is_empty());
}

// "points a dangling link at the file its plan named, and says which still resolve nowhere"
#[test]
fn a_dangling_link_is_pointed_at_the_file_its_plan_named_and_the_rest_are_named() {
  let (_root, units, deploy) = installed_units();
  let left = relink_units(
    &units,
    &deploy,
    &[UnitRelink {
      name: "app-web.service".to_owned(),
      from: deploy.join("web").join("app-web.service"),
      to: deploy.join("site").join("app-web.service"),
    }],
  )
  .unwrap();

  assert_eq!(
    std::fs::read_link(units.join("app-web.service")).unwrap(),
    deploy.join("site").join("app-web.service")
  );
  assert_eq!(std::fs::read_link(units.join("app.service")).unwrap(), deploy.join("app.service"));
  // The link into the tree the plan had no file for is still dangling; the one pointing outside the tree
  // is not the deployment's to answer for.
  assert_eq!(left, ["relative.service"]);
}

// A tree's files read by their path, as `git show` reads them for the deployment.
fn reading(files: &[(&str, &str)]) -> (Vec<String>, HashMap<String, String>) {
  let files: HashMap<String, String> =
    files.iter().map(|(path, text)| ((*path).to_owned(), (*text).to_owned())).collect();
  let mut tracked: Vec<String> = files.keys().cloned().collect();

  tracked.sort();

  (tracked, files)
}

#[test]
fn a_mark_is_the_one_line_of_the_key_and_its_word() {
  assert_eq!(UnitMark::Restart.line(), "X-BMO-Deploy=restart");
  assert_eq!(UnitMark::Bot.line(), "X-BMO-Deploy=bot");
}

// "are the unit files that carry the restart mark, and no other file"
#[test]
fn the_units_kept_standing_are_the_unit_files_that_carry_the_restart_mark() {
  let (tracked, files) = reading(&[
    (
      "app.service",
      "[Service]\nExecStart=/usr/bin/true\n\n[Install]\nWantedBy=default.target\n",
    ),
    (
      "helper.service",
      "[Service]\nKillMode=process\n\n[Install]\nWantedBy=default.target\nX-BMO-Deploy=restart\n",
    ),
    (
      "gate/relay.service",
      "[Install]\nWantedBy=default.target\n# X-BMO-Deploy=restart is named here, not set\n",
    ),
    ("units/sidecar.service", "[Install]\n  X-BMO-Deploy=restart  \n"),
    ("docs/deploy.md", "X-BMO-Deploy=restart\n"),
  ]);

  assert_eq!(
    marked_units(&tracked, &|path| files[path].clone(), ".service", UnitMark::Restart),
    ["helper.service", "units/sidecar.service"]
  );
}

// "timers that carry the timer mark are kept with the service of their own name"
#[test]
fn timers_that_carry_the_timer_mark_are_kept_with_the_service_of_their_own_name() {
  let (tracked, files) = reading(&[
    ("snapshot.timer", "[Install]\nWantedBy=timers.target\nX-BMO-Deploy=timer\n"),
    ("cleanup.timer", "[Install]\nWantedBy=timers.target\n"),
    ("helper.service", "[Install]\nX-BMO-Deploy=timer\n"),
  ]);

  assert_eq!(
    marked_units(&tracked, &|path| files[path].clone(), ".timer", UnitMark::Timer),
    ["snapshot.timer"]
  );
  assert_eq!(timer_service("units/snapshot.timer"), "units/snapshot.service");
}

// "a unit something else starts carries the link mark, and only it is listed for linking alone"
#[test]
fn a_unit_something_else_starts_carries_the_link_mark_and_only_it_is_linked_alone() {
  let (tracked, files) = reading(&[
    ("down-alert.service", "[Service]\nType=oneshot\n\n[Install]\nX-BMO-Deploy=link\n"),
    ("helper.service", "[Install]\nX-BMO-Deploy=restart\n"),
    ("app.service", "[Unit]\nOnFailure=down-alert.service\n"),
  ]);

  assert_eq!(
    marked_units(&tracked, &|path| files[path].clone(), ".service", UnitMark::Link),
    ["down-alert.service"]
  );
}

#[test]
fn the_bot_is_the_one_service_its_tree_marks_as_the_bot() {
  let (tracked, files) = reading(&[
    ("app-server.service", "[Install]\nWantedBy=default.target\nX-BMO-Deploy=bot\n"),
    ("helper.service", "[Install]\nX-BMO-Deploy=restart\n"),
  ]);

  assert_eq!(bot_unit(&tracked, &|path| files[path].clone()).as_deref(), Some("app-server"));

  let (tracked, files) = reading(&[("helper.service", "[Install]\nX-BMO-Deploy=restart\n")]);

  assert_eq!(bot_unit(&tracked, &|path| files[path].clone()), None);
}

#[test]
fn a_unit_marked_as_going_is_taken_out_only_where_it_is_installed_a_timer_before_its_service() {
  let (tracked, files) = reading(&[
    ("app.service", "[Service]\nExecStart=/bin/true\n\n[Install]\nX-BMO-Deploy=retire\n"),
    ("cleanup.service", "[Service]\nExecStart=/bin/true\n\n[Install]\nX-BMO-Deploy=retire\n"),
    ("cleanup.timer", "[Timer]\nOnActiveSec=5min\n\n[Install]\nX-BMO-Deploy=retire\n"),
    ("gone.service", "[Install]\nX-BMO-Deploy=retire\n"),
    ("helper.service", "[Install]\nX-BMO-Deploy=restart\n"),
  ]);
  let installed = names(&["app.service", "cleanup.service", "cleanup.timer", "helper.service"]);

  assert_eq!(
    retired_units(&tracked, &|path| files[path].clone(), &installed),
    ["cleanup.timer", "cleanup.service", "app.service"]
  );
}

// "a marked unit installed as a plain file is moved aside, never deleted, and a link is left alone"
#[test]
fn a_marked_unit_installed_as_a_plain_file_is_moved_aside_and_a_link_is_left_alone() {
  let units = tempfile::tempdir().unwrap();

  std::fs::write(units.path().join("snapshot.timer"), "[Timer]\nOnCalendar=hourly\n").unwrap();
  symlink("/nowhere/helper.service", units.path().join("helper.service")).unwrap();

  assert_eq!(
    set_aside_plain_units(units.path(), &names(&["snapshot.timer", "units/helper.service", "drill.timer"])),
    ["snapshot.timer"]
  );
  assert!(
    std::fs::read_to_string(units.path().join("snapshot.timer.before-deploy"))
      .unwrap()
      .contains("hourly")
  );
  assert_eq!(
    std::fs::read_link(units.path().join("helper.service")).unwrap(),
    Path::new("/nowhere/helper.service")
  );
}

// "are linked in where no unit of their name is installed, and an installed one is left as it is"
#[test]
fn the_marked_units_are_linked_in_where_no_unit_of_their_name_is_installed() {
  let root = tempfile::tempdir().unwrap();
  let units = root.path().join("units");
  let deploy = root.path().join("deploy");
  let installs = units_to_link(
    &names(&["helper.service", "units/sidecar.service"]),
    &deploy,
    &names(&["sidecar.service", "app.service"]),
  );

  assert_eq!(
    installs,
    [UnitInstall {
      name: "helper.service".to_owned(),
      to: deploy.join("helper.service"),
    }]
  );

  link_units(&units, &installs).unwrap();
  assert_eq!(std::fs::read_link(units.join("helper.service")).unwrap(), deploy.join("helper.service"));
}

#[test]
fn bin_is_marked_with_the_commit_it_was_built_from_and_the_mark_taken_away_for_none() {
  let dir = tempfile::tempdir().unwrap();
  let bin = dir.path().join("bin");

  mark_built_from(&bin, Some("abc")).unwrap();
  assert_eq!(std::fs::read_to_string(bin.join("built-from")).unwrap(), "abc\n");
  assert!(!bin.join("built-from.new").exists());
  mark_built_from(&bin, None).unwrap();
  assert!(!bin.join("built-from").exists());
  mark_built_from(&bin, None).unwrap();
}

// "units marked binary are found by their mark and restarted only where what they run changed or they
// were linked now"
#[test]
fn units_marked_binary_are_restarted_only_where_what_they_run_changed_or_they_were_linked_now() {
  let (tracked, files) = reading(&[
    (
      "gate/card.service",
      "[Service]\nExecStart=%h/apps/deploy/bin/card serve\n\n[Install]\nX-BMO-Deploy=binary\n",
    ),
    (
      "gate/relay.service",
      "[Service]\nExecStart=-%h/apps/deploy/bin/relay\n\n[Install]\nX-BMO-Deploy=binary\n",
    ),
    (
      "gateway/gateway.service",
      "[Service]\nExecStart=%h/apps/deploy/bin/gateway\n\n[Install]\nX-BMO-Deploy=binary\n",
    ),
    (
      "helper.service",
      "[Service]\nExecStart=%h/apps/deploy/bin/helper\n\n[Install]\nX-BMO-Deploy=restart\n",
    ),
  ]);
  let bound = marked_units(&tracked, &|path| files[path].clone(), ".service", UnitMark::Binary);
  let units: Vec<(String, String)> =
    bound.iter().map(|path| (base_name(path), files[path].clone())).collect();

  assert_eq!(bound, ["gate/card.service", "gate/relay.service", "gateway/gateway.service"]);
  assert_eq!(binary_restarts(&units, &names(&["relay", "helper"]), &[]), ["relay.service"]);
  assert_eq!(binary_restarts(&units, &[], &names(&["gateway.service"])), ["gateway.service"]);
  assert_eq!(binary_restarts(&units, &[], &[]), Vec::<String>::new());
  assert_eq!(unit_program("ExecStart=@/bin/x y"), Some("/bin/x".to_owned()));
}

// "a marked unit linked into a checkout is pointed at the deploy tree, and one already there is left"
#[test]
fn a_marked_unit_linked_into_a_checkout_is_pointed_at_the_deploy_tree() {
  let root = tempfile::tempdir().unwrap();
  let units = root.path().join("units");
  let deploy = root.path().join("deploy");
  let checkout = root.path().join("checkout");

  std::fs::create_dir_all(&units).unwrap();
  std::fs::create_dir_all(deploy.join("gate")).unwrap();
  std::fs::create_dir_all(checkout.join("gate")).unwrap();
  std::fs::write(deploy.join("gate").join("card.service"), "[Install]\nX-BMO-Deploy=binary\n").unwrap();
  std::fs::write(checkout.join("gate").join("card.service"), "[Install]\n").unwrap();
  std::fs::write(deploy.join("helper.service"), "[Install]\nX-BMO-Deploy=restart\n").unwrap();
  symlink(checkout.join("gate").join("card.service"), units.join("card.service")).unwrap();
  symlink(deploy.join("helper.service"), units.join("helper.service")).unwrap();
  symlink(checkout.join("gate").join("gate.service"), units.join("gate.service")).unwrap();

  let repointed =
    outside_relinks(&unit_links(&units), &names(&["gate/card.service", "helper.service"]), &deploy);

  assert_eq!(
    repointed,
    [UnitRelink {
      name: "card.service".to_owned(),
      from: checkout.join("gate").join("card.service"),
      to: deploy.join("gate").join("card.service"),
    }]
  );
  assert_eq!(relink_units(&units, &deploy, &repointed).unwrap(), Vec::<String>::new());
  assert_eq!(
    std::fs::read_link(units.join("card.service")).unwrap(),
    deploy.join("gate").join("card.service")
  );
  assert_eq!(
    std::fs::read_link(units.join("gate.service")).unwrap(),
    checkout.join("gate").join("gate.service")
  );
}
