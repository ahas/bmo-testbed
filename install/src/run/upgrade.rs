//! `upgrade` (docs/install.md, 「Upgrading: `upgrade`」): every checkout of the install moved forward to one target,
//! the newest release tag or the head of its main branch, then every installed repository deployed at its checkout's
//! target in the install's order, the run stopping at the first that does not go live.

use std::collections::BTreeMap;
use std::path::Path;

use bmo_testbed_core::declaration::Repository;
use bmo_testbed_core::policy::install::INSTALL;
use bmo_testbed_core::version::release_of_tag;

use super::{Install, InstallDeclaration, Unit, mark};
use crate::line::{Line, Status};
use crate::machine::Ran;
use crate::presets::{FILE as PRESETS_FILE, PresetsFile};

use super::Outcome;

/// What one checkout moves to: the commit, and the words that name it on a line.
struct Target {
  commit: String,
  named: String,
}

impl Unit {
  // The commit the repository's deployment record names as live.
  fn live_commit(&self, ask: &super::Ask) -> Option<String> {
    let text = std::fs::read_to_string(self.deploy_root(ask)?.join("deployed.json")).ok()?;

    serde_json::from_str::<serde_json::Value>(&text)
      .ok()?
      .get("commit")?
      .as_str()
      .map(str::to_owned)
  }

  // The main branch its testbed declaration names, `main` where it has none.
  fn main(&self) -> String {
    self
      .testbed
      .as_ref()
      .map_or_else(|| "main".to_owned(), |testbed| testbed.check.main.clone())
  }
}

impl Install<'_> {
  pub(super) fn upgrade(&mut self, index: &Path, presets: &PresetsFile, nightly: bool) -> Outcome {
    let said = &INSTALL.upgrade;
    let Some(state) = self.read_state() else {
      return Self::refused(&mark(
        &INSTALL.refusal.no_install,
        &[("dir", &self.ask.data_root.display().to_string())],
      ));
    };
    let resolved = match Self::resolved(presets, &state) {
      Ok(resolved) => resolved,
      Err(why) => return Self::refusal(why, &index.join(PRESETS_FILE)),
    };
    let rerun = mark(&said.rerun, &[("flag", if nightly { said.rerun_nightly.as_str() } else { "" })]);

    self.line(
      "target",
      Status::Ok,
      if nightly { &said.target_nightly } else { &said.target_release },
      "",
    );

    // 1. Every checkout of the install, cloned and installed alike, moved to its target.
    let mut targets: BTreeMap<String, String> = BTreeMap::new();

    for name in resolved.clone.iter().chain(&resolved.units) {
      if let Some(target) = self.move_checkout(index, presets, name, nightly, &rerun) {
        targets.insert(name.clone(), target);
      }
    }

    if let Some(step) = self.failed.clone() {
      (self.say)(&Line::new(
        "done",
        Status::Failed,
        format!("{}; {}", mark(&said.done_failed, &[("step", &step)]), said.checkouts_stopped),
        rerun,
      ));
      return self.outcome();
    }

    // 2. Every installed repository put live at its checkout's target, in the install's order.
    let message = mark(
      &said.message,
      &[("target", if nightly { &said.message_nightly } else { &said.message_release })],
    );
    let mut count = 0usize;

    for name in &resolved.units {
      let checkout = Self::checkout_of(index, presets, name);
      let unit = Unit {
        name: name.clone(),
        testbed: Repository::read(&checkout).ok().map(|repository| repository.declaration),
        checkout,
        declaration: InstallDeclaration::default(),
      };
      let Some(commit) = targets.get(name) else { continue };

      count += 1;

      if !self.redeploy(&unit, commit, &message, &rerun) {
        break;
      }
    }

    let count = count.to_string();

    if let Some(step) = self.failed.clone() {
      (self.say)(&Line::new("done", Status::Failed, mark(&said.done_failed, &[("step", &step)]), rerun));
    } else if self.ask.dry_run {
      (self.say)(&Line::new("done", Status::Planned, mark(&said.done_planned, &[("count", &count)]), ""));
    } else {
      (self.say)(&Line::new("done", Status::Ok, mark(&said.done, &[("count", &count)]), ""));
    }

    self.outcome()
  }

  // One checkout fetched and moved forward to its target; the target's commit, or none where the line failed.
  fn move_checkout(
    &mut self,
    index: &Path,
    presets: &PresetsFile,
    name: &str,
    nightly: bool,
    rerun: &str,
  ) -> Option<String> {
    let said = &INSTALL.upgrade;
    let checkout = Self::checkout_of(index, presets, name);
    let step = format!("checkout:{name}");
    let path = checkout.display().to_string();

    if !checkout.join(".git").exists() {
      self.line(
        step,
        Status::Failed,
        mark(&said.checkout_absent, &[("name", name), ("path", &path)]),
        rerun,
      );
      return None;
    }

    if !self.ask.dry_run {
      let fetched = self.machine.fetch(&checkout);

      if !fetched.ok() {
        let target = if nightly { "origin" } else { "its tags" };

        self.line(
          step,
          Status::Failed,
          mark(
            &said.checkout_failed,
            &[("name", name), ("target", target), ("said", fetched.said.trim())],
          ),
          rerun,
        );
        return None;
      }
    }

    if self.machine.has_changes(&checkout) {
      self.line(
        step,
        Status::Failed,
        mark(&said.checkout_dirty, &[("name", name), ("path", &path)]),
        rerun,
      );
      return None;
    }

    let unit = Unit {
      name: name.to_owned(),
      testbed: Repository::read(&checkout).ok().map(|repository| repository.declaration),
      checkout: checkout.clone(),
      declaration: InstallDeclaration::default(),
    };
    let main = unit.main();
    let branch = self.machine.branch(&checkout);
    let target = if nightly {
      if let Some(branch) = branch.as_deref().filter(|branch| *branch != main) {
        self.line(
          step,
          Status::Failed,
          mark(&said.checkout_branch, &[("name", name), ("branch", branch), ("main", &main)]),
          rerun,
        );
        return None;
      }

      let Some(commit) = self.machine.main_head(&checkout, &main) else {
        self.line(
          step,
          Status::Failed,
          mark(
            &said.checkout_failed,
            &[("name", name), ("target", &format!("origin/{main}")), ("said", "it does not resolve")],
          ),
          rerun,
        );
        return None;
      };

      Target {
        commit,
        named: mark(&said.nightly, &[("main", &main)]),
      }
    } else {
      let newest = self
        .machine
        .tags(&checkout)
        .into_iter()
        .filter_map(|tag| release_of_tag(&tag).map(|version| (version, tag)))
        .max();
      let Some((_, tag)) = newest else {
        self.line(step, Status::Failed, mark(&said.checkout_no_release, &[("name", name)]), rerun);
        return None;
      };
      let Some(commit) = self.machine.commit_of(&checkout, &tag) else {
        self.line(
          step,
          Status::Failed,
          mark(
            &said.checkout_failed,
            &[("name", name), ("target", &tag), ("said", "it does not resolve")],
          ),
          rerun,
        );
        return None;
      };

      Target {
        commit,
        named: mark(&said.release, &[("tag", &tag)]),
      }
    };
    let current = self.machine.commit_of(&checkout, "HEAD").unwrap_or_default();
    let at = [("name", name), ("target", target.named.as_str()), ("commit", target.commit.as_str())];

    // Already there, on the branch the target asks for where it asks for one.
    if current == target.commit && (!nightly || branch.is_some()) {
      self.line(step, Status::Ok, mark(&said.checkout_stands, &at), "");
      return Some(target.commit);
    }

    if self.ask.dry_run {
      let from = if current.is_empty() { "nothing" } else { current.as_str() };

      self.line(
        step,
        Status::Planned,
        mark(
          &said.checkout_planned,
          &[("name", name), ("from", from), ("target", &target.named), ("commit", &target.commit)],
        ),
        "",
      );
      return Some(target.commit);
    }

    let moved = if nightly {
      let onto = if branch.is_none() {
        self.machine.check_out(&checkout, &main)
      } else {
        Ran {
          code: 0,
          said: String::new(),
        }
      };

      if onto.ok() {
        let forward = self.machine.fast_forward(&checkout, &main);

        if !forward.ok() {
          self.line(
            step,
            Status::Failed,
            mark(
              &said.checkout_diverged,
              &[("name", name), ("main", &main), ("said", forward.said.trim())],
            ),
            rerun,
          );
          return None;
        }
      }

      onto
    } else {
      self.machine.check_out(&checkout, &target.commit)
    };

    if !moved.ok() || self.machine.commit_of(&checkout, "HEAD").as_deref() != Some(target.commit.as_str()) {
      self.line(
        step,
        Status::Failed,
        mark(
          &said.checkout_failed,
          &[("name", name), ("target", &target.named), ("said", moved.said.trim())],
        ),
        rerun,
      );
      return None;
    }

    self.line(step, Status::Ok, mark(&said.checkout_moved, &at), "");
    Some(target.commit)
  }

  // One installed repository put live at the commit; false where the run stops here.
  fn redeploy(&mut self, unit: &Unit, commit: &str, message: &str, rerun: &str) -> bool {
    let said = &INSTALL.upgrade;
    let step = format!("deploy:{}", unit.name);
    let at = [("name", unit.name.as_str()), ("commit", commit)];

    if unit.deploy_tree(&self.ask).is_none() {
      self.line(step, Status::Ok, mark(&INSTALL.detail.deploy_none, &[("name", &unit.name)]), "");
      return true;
    }

    if unit.deployed(&self.ask) && unit.live_commit(&self.ask).as_deref() == Some(commit) {
      self.line(step, Status::Ok, mark(&said.deploy_live, &at), "");
      return true;
    }

    if self.ask.dry_run {
      self.line(step, Status::Planned, mark(&said.deploy_planned, &at), "");
      return true;
    }

    let (ran, deployment) = self.machine.deploy(&unit.checkout, commit, message, false);

    if ran.ok() && unit.live_commit(&self.ask).as_deref() == Some(commit) && unit.deployed(&self.ask) {
      self.line(step, Status::Ok, mark(&said.deploy_done, &at), "");
      return true;
    }

    let why = if ran.ok() {
      unit.live_commit(&self.ask).map_or_else(
        || "no deployment record names it".to_owned(),
        |live| format!("the record names {live}"),
      )
    } else {
      ran.said.trim().to_owned()
    };
    let next =
      deployment.map_or_else(|| rerun.to_owned(), |unit| mark(&INSTALL.next.journal, &[("unit", &unit)]));

    self.line(
      step,
      Status::Failed,
      mark(&said.deploy_failed, &[("name", &unit.name), ("commit", commit), ("said", &why)]),
      next,
    );
    false
  }
}
