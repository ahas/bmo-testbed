//! `pack` (docs/artifacts.md): the repository the verb runs in packed at a commit, each line it says printed as it
//! comes, the manifest printed last.

use std::path::PathBuf;

use bmo_testbed_deploy::pack::{PackAsk, pack as packed};
use bmo_testbed_suite::crates::checkout_target_dir;

use crate::words::PackArgs;

pub fn pack(args: &PackArgs) -> u8 {
  let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
  let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"));
  let ask = PackAsk {
    target_dir: checkout_target_dir(&cwd).join("pack"),
    checkout: cwd.clone(),
    commit: args.commit.clone(),
    out: args.out.clone().unwrap_or(cwd),
    key: args.key.clone(),
    cache: home.join(".cache/bmo-pack"),
  };

  match packed(&ask, &|line| println!("{line}")) {
    Ok(done) => {
      println!("{}", serde_json::to_string(&done.manifest_json).unwrap_or_default());
      0
    }
    Err(why) => {
      eprintln!("{why}");
      1
    }
  }
}
