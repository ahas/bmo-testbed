//! The releases (docs/deploy.md, 「The releases」): where they stand beside the link, a release made or
//! taken as it stands, the flip, the binaries placed, the live processes read off `/proc`, and a prune.

use std::path::{Path, PathBuf};

use bmo_testbed_deploy::expansion::expansion_tree;
use bmo_testbed_deploy::release::{
  LiveProcess, Preparing, Pruned, Standing, differing_binaries, flip, is_deployment_tree, link_of,
  live_processes, live_release, live_release_real, logged_before, pinned_detail, place_binaries, prepare,
  prune, rebuilt_beside, release_dir, release_holding, releases_dir, standing,
};
use bmo_testbed_deploy::tree::short;

use crate::stand::{git, mark_built, must, sleeper, stand};

#[test]
fn the_releases_stand_beside_the_deploy_tree() {
  let deploy = Path::new("/srv/app/deploy");

  assert_eq!(releases_dir(deploy), Path::new("/srv/app/deploy-releases"));
  assert_eq!(
    release_dir(deploy, &"0123456789abcdef".repeat(2)),
    Path::new("/srv/app/deploy-releases/0123456789ab")
  );
}

#[test]
fn the_releases_and_the_build_tree_are_the_deployments_and_a_checkout_beside_them_is_not() {
  let root = tempfile::tempdir().unwrap();
  let deploy = root.path().join("deploy");
  let release = root.path().join("deploy-releases").join("0123456789ab");
  let other = root.path().join("deploy-releases").join("ba9876543210");
  let checkout = root.path().join("app-w1");

  for dir in [&release, &other, &root.path().join("deploy-expand"), &checkout] {
    std::fs::create_dir_all(dir).unwrap();
  }

  std::os::unix::fs::symlink("deploy-releases/0123456789ab", &deploy).unwrap();

  assert!(is_deployment_tree(&deploy, &deploy));
  assert!(is_deployment_tree(&deploy, &release));
  assert!(is_deployment_tree(&deploy, &other));
  assert!(is_deployment_tree(&deploy, &root.path().join("deploy-expand")));
  assert!(!is_deployment_tree(&deploy, &checkout));
  assert!(!is_deployment_tree(&deploy, &release.join("app")));
}

#[test]
fn a_release_named_in_the_links_place_finds_what_stands_beside_the_link() {
  let release = Path::new("/srv/app/deploy-releases/0123456789ab");

  assert_eq!(link_of(release), Path::new("/srv/app/deploy"));
  assert_eq!(link_of(&release.join("bin")), release.join("bin"));
  assert_eq!(link_of(Path::new("/srv/app/deploy")), Path::new("/srv/app/deploy"));
  assert_eq!(
    link_of(Path::new("/srv/app/-releases/0123456789ab")),
    Path::new("/srv/app/-releases/0123456789ab")
  );
  assert_eq!(releases_dir(release), Path::new("/srv/app/deploy-releases"));
  assert_eq!(expansion_tree(release), Path::new("/srv/app/deploy-expand"));
}

#[test]
fn the_release_a_path_lies_in_is_read_by_its_real_path_through_the_link() {
  let root = tempfile::tempdir().unwrap();
  let deploy = root.path().join("deploy");
  let release = root.path().join("deploy-releases").join("0123456789ab");
  let checkout = root.path().join("app-w1");

  for dir in [&release.join("bin"), &checkout] {
    std::fs::create_dir_all(dir).unwrap();
  }

  std::fs::write(release.join("bin").join("app"), "").unwrap();
  std::os::unix::fs::symlink("deploy-releases/0123456789ab", &deploy).unwrap();

  let real = std::fs::canonicalize(&release).unwrap();

  assert_eq!(release_holding(&deploy, &deploy.join("bin").join("app")), Some(real.clone()));
  assert_eq!(release_holding(&release, &release.join("bin")), Some(real.clone()));
  assert_eq!(release_holding(&deploy, &checkout), None);
  assert_eq!(live_release_real(&deploy), Some(real));
  assert_eq!(live_release_real(&checkout), None);
}

// A deploy tree's link onto one release among three, with the build tree and a retired tree beside it.
fn release_layout() -> (tempfile::TempDir, PathBuf) {
  let root = tempfile::tempdir().unwrap();
  let app = root.path().join("app");

  for name in [
    "deploy-releases/0123456789ab",
    "deploy-releases/111111111111",
    "deploy-releases/222222222222",
    "deploy-releases/.incoming",
    "deploy-expand",
    "deploy.retired-20261014-070000",
    "other",
  ] {
    std::fs::create_dir_all(app.join(name)).unwrap();
  }

  std::os::unix::fs::symlink("deploy-releases/0123456789ab", app.join("deploy")).unwrap();

  (root, app.join("deploy"))
}

#[test]
fn what_a_deployment_makes_again_beside_the_link_is_left_out_and_the_live_release_is_not() {
  let (root, deploy) = release_layout();
  let app = root.path().join("app");

  assert_eq!(
    rebuilt_beside(&deploy),
    [
      "deploy-expand",
      "deploy-releases/111111111111",
      "deploy-releases/222222222222",
      "deploy.retired-20261014-070000",
    ]
    .map(|name| app.join(name))
    .to_vec(),
  );
  // A reader pinned to a release asks where the link is and gets the same list.
  assert_eq!(rebuilt_beside(&app.join("deploy-releases/0123456789ab")), rebuilt_beside(&deploy));
}

#[test]
fn a_deploy_tree_that_is_still_a_directory_leaves_nothing_new_out() {
  let root = tempfile::tempdir().unwrap();
  let deploy = root.path().join("deploy");

  std::fs::create_dir_all(deploy.join("bin")).unwrap();

  assert!(rebuilt_beside(&deploy).is_empty());
}

#[test]
fn a_release_is_made_as_a_worktree_at_its_commit_and_set_up_once() {
  let at = stand();
  let head = at.head();
  let setups = std::cell::Cell::new(0);
  let prepared = at.prepare(&head, &setups);

  assert!(prepared.made);
  assert_eq!(prepared.release, release_dir(&at.deploy_dir, &head));
  assert_eq!(must(&["rev-parse", "HEAD"], &prepared.release), head);
  assert_eq!(setups.get(), 1);
}

#[test]
fn a_release_standing_built_is_taken_as_it_stands_and_an_unfinished_one_is_made_again() {
  let at = stand();
  let head = at.head();
  let setups = std::cell::Cell::new(0);
  let first = at.prepare(&head, &setups);

  // Its build never finished: no `built-from`, so it is made afresh.
  std::fs::write(first.release.join("left.txt"), "left by a cut-off run").unwrap();
  assert!(at.prepare(&head, &setups).made);
  assert!(!first.release.join("left.txt").exists());

  mark_built(&first.release, &head);

  let again = at.prepare(&head, &setups);

  assert!(!again.made);
  assert_eq!(setups.get(), 2);
}

#[test]
fn the_live_release_is_never_made_again_under_what_runs() {
  let at = stand();
  let head = at.head();
  let setups = std::cell::Cell::new(0);
  let live = at.prepare(&head, &setups).release;

  flip(&at.deploy_dir, &live).unwrap();

  let preparing = Preparing {
    source: &at.source,
    deploy_dir: &at.deploy_dir,
    commit: &head,
    data_root: &at.data_root,
  };
  let refused = prepare(&preparing, &git, &|_| Ok(()), &|_| {}).unwrap_err();

  assert!(refused.contains("not made again"), "{refused}");
  assert!(live.join(".git").exists());
}

#[test]
fn the_flip_renames_a_relative_link_over_the_deploy_tree_and_back() {
  let at = stand();
  let first = at.head();
  let setups = std::cell::Cell::new(0);
  let old = at.prepare(&first, &setups).release;
  let second = at.commit("second.txt");
  let new = at.prepare(&second, &setups).release;

  assert_eq!(standing(&at.deploy_dir), Standing::Nothing);
  flip(&at.deploy_dir, &old).unwrap();
  assert_eq!(live_release(&at.deploy_dir), Some(old.clone()));
  flip(&at.deploy_dir, &new).unwrap();
  assert_eq!(
    std::fs::read_link(&at.deploy_dir).unwrap(),
    Path::new("deploy-releases").join(short(&second))
  );
  assert!(at.deploy_dir.join("second.txt").exists());

  // The flip back is the same rename the other way.
  flip(&at.deploy_dir, &old).unwrap();
  assert!(!at.deploy_dir.join("second.txt").exists());
  assert_eq!(
    std::fs::read_dir(at.root.path())
      .unwrap()
      .flatten()
      .filter(|entry| entry.file_name().to_string_lossy().contains(".flip-"))
      .count(),
    0
  );
}

#[test]
fn a_deploy_tree_standing_as_a_directory_is_never_renamed_over_by_the_flip() {
  let at = stand();

  std::fs::create_dir_all(at.deploy_dir.join("bin")).unwrap();

  assert_eq!(standing(&at.deploy_dir), Standing::NotLink);
  assert!(flip(&at.deploy_dir, &at.source).is_err());
  assert!(at.deploy_dir.join("bin").is_dir());
}

#[test]
fn a_binary_with_the_previous_releases_bytes_is_hard_linked_and_one_that_differs_copied() {
  use std::os::unix::fs::MetadataExt;

  let dir = tempfile::tempdir().unwrap();
  let built = dir.path().join("built");
  let previous = dir.path().join("previous/bin");
  let bin = dir.path().join("release/bin");

  std::fs::create_dir_all(&built).unwrap();
  std::fs::create_dir_all(&previous).unwrap();
  std::fs::write(built.join("server"), "same").unwrap();
  std::fs::write(previous.join("server"), "same").unwrap();
  std::fs::write(built.join("mover"), "new").unwrap();
  std::fs::write(previous.join("mover"), "old").unwrap();
  std::fs::write(built.join("router"), "first").unwrap();

  let names = ["mover".to_owned(), "router".to_owned(), "server".to_owned()];
  let changed = place_binaries(&names, &built, Some(&previous), &bin).unwrap();

  assert_eq!(changed, ["mover", "router"]);
  assert_eq!(
    std::fs::metadata(bin.join("server")).unwrap().ino(),
    std::fs::metadata(previous.join("server")).unwrap().ino()
  );
  assert_ne!(
    std::fs::metadata(bin.join("mover")).unwrap().ino(),
    std::fs::metadata(previous.join("mover")).unwrap().ino()
  );
  assert_eq!(std::fs::read_to_string(previous.join("mover")).unwrap(), "old");
  assert_eq!(std::fs::metadata(bin.join("router")).unwrap().mode() & 0o777, 0o755);
  assert_eq!(
    differing_binaries(&dir.path().join("previous"), &dir.path().join("release")),
    ["mover", "router"]
  );
}

#[test]
fn a_prune_keeps_the_releases_named_and_takes_the_rest_with_their_worktrees() {
  let at = stand();
  let setups = std::cell::Cell::new(0);
  let one = at.prepare(&at.head(), &setups).release;
  let two = at.prepare(&at.commit("two.txt"), &setups).release;
  let three = at.prepare(&at.commit("three.txt"), &setups).release;

  std::fs::create_dir_all(releases_dir(&at.deploy_dir).join(".incoming")).unwrap();

  let pruned = prune(
    &at.deploy_dir,
    &[two.clone(), three.clone()],
    Some(&[]),
    &at.source,
    &at.data_root,
    &git,
  );

  assert_eq!(pruned.removed, [one.file_name().unwrap().to_string_lossy()]);
  assert!(pruned.warnings.is_empty());
  assert!(!one.exists() && two.exists() && three.exists());
  assert!(releases_dir(&at.deploy_dir).join(".incoming").exists());
  assert!(!must(&["worktree", "list"], &at.source).contains(&one.to_string_lossy().into_owned()));
}

fn live() -> Vec<LiveProcess> {
  live_processes(Path::new("/proc")).unwrap()
}

#[test]
fn a_prune_keeps_a_release_a_live_process_stands_in_and_takes_it_once_the_process_ends() {
  let at = stand();
  let setups = std::cell::Cell::new(0);
  let worked_in = at.prepare(&at.head(), &setups).release;
  let named = at.prepare(&at.commit("two.txt"), &setups).release;
  let free = at.prepare(&at.commit("three.txt"), &setups).release;
  let live_one = at.prepare(&at.commit("four.txt"), &setups).release;
  let in_it = sleeper(&worked_in.join("app"), None);
  let naming_it = sleeper(at.root.path(), Some(&named));
  let name = |release: &Path| release.file_name().unwrap().to_string_lossy().into_owned();
  let both = || {
    let mut both = vec![name(&worked_in), name(&named)];

    both.sort();
    both
  };
  let pruned = prune(
    &at.deploy_dir,
    std::slice::from_ref(&live_one),
    Some(&live()),
    &at.source,
    &at.data_root,
    &git,
  );

  assert_eq!(pruned.removed, [name(&free)]);
  assert_eq!(pruned.standing, 3);
  assert_eq!(pruned.pinned.iter().map(|(release, _)| release.clone()).collect::<Vec<_>>(), both());

  let holders = |release: &Path| {
    pruned
      .pinned
      .iter()
      .find(|(held, _)| *held == name(release))
      .map(|(_, processes)| processes.iter().map(|process| process.pid).collect::<Vec<_>>())
  };

  assert_eq!(holders(&worked_in), Some(vec![in_it.0.id()]));
  assert_eq!(holders(&named), Some(vec![naming_it.0.id()]));
  assert!(worked_in.exists() && named.exists() && live_one.exists() && !free.exists());

  drop(in_it);
  drop(naming_it);

  let pruned = prune(
    &at.deploy_dir,
    std::slice::from_ref(&live_one),
    Some(&live()),
    &at.source,
    &at.data_root,
    &git,
  );

  assert_eq!(pruned.removed, both());
  assert!(pruned.pinned.is_empty());
  assert_eq!(pruned.standing, 1);
  assert!(!worked_in.exists() && !named.exists() && live_one.exists());
}

#[test]
fn a_prune_takes_nothing_where_the_live_processes_could_not_be_read() {
  let at = stand();
  let setups = std::cell::Cell::new(0);
  let one = at.prepare(&at.head(), &setups).release;
  let pruned = prune(&at.deploy_dir, &[], None, &at.source, &at.data_root, &git);

  assert!(pruned.removed.is_empty());
  assert_eq!(pruned.warnings.len(), 1);
  assert!(one.exists());
}

#[test]
fn a_prune_never_removes_a_release_inside_the_data_root() {
  let at = stand();
  let setups = std::cell::Cell::new(0);
  let one = at.prepare(&at.head(), &setups).release;
  let pruned = prune(&at.deploy_dir, &[], Some(&[]), &at.source, at.root.path(), &git);

  assert!(pruned.removed.is_empty());
  assert!(pruned.warnings[0].contains("inside the runtime data root"), "{:?}", pruned.warnings);
  assert!(one.exists());
}

fn process(unit: &str) -> LiveProcess {
  LiveProcess {
    pid: 1,
    unit: Some(unit.to_owned()),
    command: "runner".to_owned(),
    cwd: Some(PathBuf::from("/")),
    exe: Some(PathBuf::from("/usr/bin/runner")),
    names_deploy_dir: false,
    named_tree: None,
    read: true,
  }
}

#[test]
fn the_reader_reads_each_process_of_this_account_for_its_places_its_variable_and_its_unit() {
  let root = tempfile::tempdir().unwrap();
  let proc_root = root.path().join("proc");
  let release = root.path().join("deploy-releases/0123456789ab");
  let link = root.path().join("deploy");

  std::fs::create_dir_all(&release).unwrap();
  std::os::unix::fs::symlink("deploy-releases/0123456789ab", &link).unwrap();

  let write = |pid: &str, cgroup: &str, environ: Option<String>| {
    let dir = proc_root.join(pid);

    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("cgroup"), cgroup).unwrap();
    std::fs::write(dir.join("stat"), format!("{pid} (run job) S 1 1 1")).unwrap();
    std::fs::write(dir.join("cmdline"), "runner\0run\0job.ts\0").unwrap();
    std::os::unix::fs::symlink(std::fs::canonicalize(&release).unwrap(), dir.join("cwd")).unwrap();
    std::os::unix::fs::symlink("/usr/bin/runner", dir.join("exe")).unwrap();

    if let Some(environ) = environ {
      std::fs::write(dir.join("environ"), environ).unwrap();
    }
  };

  write(
    "41",
    "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-job-7.service\n",
    Some(format!("HOME=/h\0BMO_DEPLOY_DIR={}\0", release.display())),
  );
  write(
    "42",
    "0::/user.slice/user-1000.slice/session-3.scope\n",
    Some(format!("BMO_DEPLOY_DIR={}\0", link.display())),
  );
  write("43", "0::/\n", None);
  write("44", "0::/\n", None);
  std::fs::write(proc_root.join("44/stat"), "44 (sleep) Z 1 1 1").unwrap();
  std::fs::create_dir_all(proc_root.join("self")).unwrap();

  let read = live_processes(&proc_root).unwrap();
  let real = std::fs::canonicalize(&release).unwrap();

  assert_eq!(read.iter().map(|process| process.pid).collect::<Vec<_>>(), [41, 42, 43]);
  assert_eq!(read[0].unit.as_deref(), Some("app-job-7"));
  assert_eq!(read[0].command, "runner run job.ts");
  assert!(
    read[0].names_deploy_dir && read[0].named_tree.as_deref() == Some(real.as_path()) && read[0].read
  );
  assert_eq!(read[1].unit.as_deref(), Some("session-3"));
  assert!(read[1].names_deploy_dir && read[1].named_tree.is_none());
  assert_eq!(read[2].unit, None);
  assert!(!read[2].names_deploy_dir && !read[2].read);
  assert!(read.iter().all(|process| process.stands_in(&release)));
  assert!(!read[0].stands_in(root.path().join("elsewhere").as_path()));
  assert!(live_processes(&root.path().join("absent")).is_none());
}

#[test]
fn the_commit_before_the_live_one_is_the_last_other_the_log_names() {
  let log = "2026-10-01T00:00:00Z aaaa app-1 first\n\
             2026-10-02T00:00:00Z bbbb app-2 second\n\
             2026-10-03T00:00:00Z cccc app-3 third\n\
             2026-10-04T00:00:00Z bbbb app-4 rollback of cccc: second\n";

  assert_eq!(logged_before(log, "bbbb").as_deref(), Some("cccc"));
  assert_eq!(logged_before(log, "cccc").as_deref(), Some("bbbb"));
  assert_eq!(logged_before("", "bbbb"), None);
}

#[test]
fn more_than_twelve_releases_standing_is_the_releases_pinned_detail() {
  let mut pruned = Pruned {
    standing: 12,
    ..Pruned::default()
  };

  assert!(pinned_detail(&pruned).is_none());

  pruned.standing += 1;
  pruned.pinned.push(("0123456789ab".to_owned(), vec![process("app-job-7")]));

  let detail = pinned_detail(&pruned).unwrap();

  assert_eq!(detail["releases"][0]["release"], "0123456789ab");
  assert_eq!(detail["releases"][0]["processes"][0]["pid"], 1);
  assert_eq!(detail["releases"][0]["processes"][0]["unit"], "app-job-7");
  assert_eq!(detail["releases"][0]["processes"][0]["command"], "runner");
}
