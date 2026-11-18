//! The placed machine's testbed kept to this machine's live testbed release (docs/boundary.md, 「The placed
//! machine」; docs/testbed.md, 「A run on another machine」): which release is live, what that machine's testbed
//! names, bringing it there under one lock, the wait of a run for a machine that does not answer, the run that
//! ends as not run instead of running here with the event it sends, and what `status` reads of that machine.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use bmo_testbed_core::events::Event;
use bmo_testbed_core::placement::PlaceAsk;
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::lanes::LANES;
use bmo_testbed_core::queue::{HeldLock, LockWait, take_lock};
use bmo_testbed_core::turn::NOT_RUN_EXIT_CODE;
use bmo_testbed_deploy::release::{built_from, link_of, live_release_real};
use bmo_testbed_deploy::tree::{BUILT_FROM, short};
use serde::Serialize;
use serde_json::json;

use super::outage::Outage;
use super::{MachinePlacement, far_path, laid_over, said, succeeded};

/// The lock every bringing of the placed machine's testbed to a release holds, under the data root's `locks/`.
pub const PLACED_TESTBED_LOCK: &str = "placed-testbed.lock";

/// The release this machine's testbed deploy tree names now: its directory, and the commit it was built from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveRelease {
  pub dir: PathBuf,
  pub commit: String,
}

/// The deploy tree the running binary's release stands beside, the link each deployment of the testbed moves onto
/// the release it puts live; none where the binary stands in no release, such as a checkout's own build.
pub fn running_deploy_tree() -> Option<PathBuf> {
  let binary = std::env::current_exe().ok()?;
  let release = binary.parent()?.parent()?;
  let link = link_of(release);

  (link != release).then_some(link)
}

/// How a run waits for a placed machine that does not answer: how often it asks again, and how long at most
/// before it ends as not run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FarWait {
  pub pause: Duration,
  pub bound: Duration,
}

impl FarWait {
  /// The lanes policy's wait.
  pub fn policy() -> Self {
    let numbers = &LANES.numbers;

    Self {
      pause: Duration::from_secs(numbers.unreached_pause_seconds),
      bound: Duration::from_secs(numbers.unreached_wait_minutes * 60),
    }
  }
}

/// The placed machine as `status` reads it (docs/boundary.md, 「`status`」): whether it answers and what `ssh`
/// said where it does not, when the outage the runs placed there remember began, the commit its testbed names,
/// the live testbed release's, and whether they are one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MachineState {
  pub machine: String,
  pub reached: bool,
  pub said: Option<String>,
  pub outage_since: Option<String>,
  pub release: Option<String>,
  pub live: Option<String>,
  pub current: bool,
}

impl MachineState {
  /// The line `status` prints for the machine.
  pub fn line(&self) -> String {
    let words = &LANES.status;
    let short_of = |commit: &Option<String>| commit.as_deref().map(short).unwrap_or_default();
    let (release, live) = (short_of(&self.release), short_of(&self.live));
    let state = match (self.reached, &self.release, &self.live) {
      (false, ..) => {
        let since = self
          .outage_since
          .as_deref()
          .map(|since| fill_marks(&words.machine_outage, &[("since", since)]))
          .unwrap_or_default();

        fill_marks(
          &words.machine_unreached,
          &[("said", self.said.as_deref().unwrap_or("")), ("outage", &since)],
        )
      }
      (true, _, None) => fill_marks(&words.machine_uncompared, &[("release", &release)]),
      (true, None, Some(_)) => fill_marks(&words.machine_unreleased, &[("live", &live)]),
      (true, Some(_), Some(_)) if self.current => {
        fill_marks(&words.machine_current, &[("release", &release)])
      }
      (true, Some(_), Some(_)) => {
        fill_marks(&words.machine_behind, &[("release", &release), ("live", &live)])
      }
    };

    fill_marks(&words.machine, &[("machine", &self.machine), ("state", &state)])
  }
}

impl MachinePlacement {
  /// The live testbed release, read at the moment of asking; none where this testbed runs from no release or the
  /// deploy tree names none that was built.
  pub fn live(&self) -> Option<LiveRelease> {
    let dir = live_release_real(self.deploy_tree.as_deref()?)?;

    Some(LiveRelease {
      commit: built_from(&dir)?,
      dir,
    })
  }

  /// Whether the testbed there is the live testbed release (docs/boundary.md, 「The placed machine」); the line
  /// naming both commits where it is not, or where none reads there. Nothing is compared where no release is live.
  pub fn live_there(&self) -> Result<(), String> {
    self.live().map_or(Ok(()), |live| self.compared(&live))
  }

  // Whether the testbed there names the live release's commit; the `stale` or `unreleased` line where it does not.
  fn compared(&self, live: &LiveRelease) -> Result<(), String> {
    let words = &LANES.placed;
    let machine = self.settings.label();

    match self.far_built_from() {
      Ok(far) if far == live.commit => Ok(()),
      Ok(far) => Err(fill_marks(
        &words.stale,
        &[("machine", machine), ("far", &short(&far)), ("live", &short(&live.commit))],
      )),
      Err(said) => Err(fill_marks(
        &words.unreleased,
        &[
          ("machine", machine),
          ("file", &self.far_built_from_file()),
          ("said", &said),
          ("live", &short(&live.commit)),
        ],
      )),
    }
  }

  // The `built-from` beside the testbed binary there.
  fn far_built_from_file(&self) -> String {
    Path::new(&self.settings.testbed)
      .with_file_name(BUILT_FROM)
      .to_string_lossy()
      .into_owned()
  }

  // The commit the `built-from` there names; what reading it said where it names none.
  fn far_built_from(&self) -> Result<String, String> {
    let read = self.ssh().arg(format!("cat {}", far_path(&self.far_built_from_file()))).output();

    read
      .as_ref()
      .ok()
      .filter(|done| done.status.success())
      .map(|done| String::from_utf8_lossy(&done.stdout).trim().to_owned())
      .filter(|commit| !commit.is_empty())
      .ok_or_else(|| said(&read))
  }

  /// The binary of a release the testbed there would run: the file in the release's `bin/` of the name the
  /// settings' `testbed` ends in; none where the release holds no such file, so it carries no testbed.
  pub fn testbed_in(&self, release: &Path) -> Option<PathBuf> {
    Some(release.join(self.testbed_name())).filter(|binary| binary.is_file())
  }

  // The name the settings' `testbed` ends in, under `bin/`.
  fn testbed_name(&self) -> String {
    format!(
      "bin/{}",
      Path::new(&self.settings.testbed).file_name().unwrap_or_default().to_string_lossy()
    )
  }

  // The lock every bringing of the testbed there holds, taken within the policy's time; the line saying it was not.
  fn held_lock(&self) -> Result<HeldLock, String> {
    let locks = self.data_root.join("locks");
    let lock = locks.join(PLACED_TESTBED_LOCK);
    let seconds = LANES.numbers.placed_lock_seconds;
    let wait = LockWait {
      holder: format!("bringing the testbed on {} ({})", self.settings.label(), std::process::id()),
      ..LockWait::default()
    };
    let _ = std::fs::create_dir_all(&locks);

    take_lock(&lock, seconds as f64, wait).ok_or_else(|| {
      fill_marks(
        &LANES.placed.lock_unheld,
        &[("file", &lock.to_string_lossy()), ("seconds", &seconds.to_string())],
      )
    })
  }

  /// The testbed there brought to a release's (docs/boundary.md, 「The placed machine」), under the lock: the
  /// release's `binary` written onto the settings' `testbed` and run there once, the `built-from` beside it onto
  /// the one beside that, last, and that `built-from` read back, which must name `commit`. The files written
  /// there; the reason where the lock was not had, the machine is not reached, a file is not written, what is read
  /// back names another commit or none, or the testbed there does not run.
  pub fn bring_testbed(&self, binary: &Path, commit: &str) -> Result<Vec<String>, String> {
    let _held = self.held_lock()?;

    self.copied(binary, commit)
  }

  // The testbed there brought to a release's, the lock taken already.
  fn copied(&self, binary: &Path, commit: &str) -> Result<Vec<String>, String> {
    let words = &LANES.placed;
    let probed = self.ssh().arg("true").output();

    if !succeeded(&probed) {
      return Err(fill_marks(&words.testbed_unreached, &[("said", &said(&probed))]));
    }

    let unwritten =
      |file: &str, why: &str| fill_marks(&words.testbed_unwritten, &[("file", file), ("said", why)]);
    let far_testbed = self.settings.testbed.clone();
    let far_built_from = self.far_built_from_file();
    let far_bin = Path::new(&far_testbed)
      .parent()
      .map(|bin| bin.to_string_lossy().into_owned())
      .unwrap_or_default();

    if !far_bin.is_empty() {
      let made = self.ssh().arg(format!("mkdir -p {}", far_path(&far_bin))).output();

      if !succeeded(&made) {
        return Err(unwritten(&far_bin, &said(&made)));
      }
    }

    let (program, options) = self.ssh_program();
    let lay = |from: &Path, to: &str, mode: Option<&str>| {
      let bytes = std::fs::read(from).map_err(|why| unwritten(to, &why.to_string()))?;
      let laid = laid_over(program, options, &self.settings.destination, to, &bytes, mode);

      if succeeded(&laid) { Ok(()) } else { Err(unwritten(to, &said(&laid))) }
    };

    lay(binary, &far_testbed, Some("755"))?;

    // The copied binary asked its help there before the `built-from` beside it names the commit, so one that does
    // not run on that machine is found now and the machine never reads as brought.
    let ran = self.ssh().arg(format!("{} --help > /dev/null", far_path(&far_testbed))).output();

    if !succeeded(&ran) {
      return Err(fill_marks(&words.testbed_unrun, &[("said", &said(&ran))]));
    }

    lay(&binary.with_file_name(BUILT_FROM), &far_built_from, None)?;

    match self.far_built_from() {
      Ok(far) if far == commit => {}
      Ok(far) => {
        return Err(fill_marks(&words.testbed_other, &[("far", &short(&far)), ("live", &short(commit))]));
      }
      Err(why) => {
        return Err(fill_marks(&words.testbed_unread, &[("file", &far_built_from), ("said", &why)]));
      }
    }

    Ok(vec![far_testbed, far_built_from])
  }

  /// The testbed there brought to the live testbed release before anything runs there (docs/testbed.md, 「A run on
  /// another machine」), where it is not that release already, said on a line; the reason naming both commits and
  /// what the machine answered where it could not be. The live release is read again once the lock is held, so a
  /// release no longer live is never copied over a newer one.
  pub fn brought_live(&self) -> Result<(), String> {
    if self.live_there().is_ok() {
      return Ok(());
    }

    let _held = self.held_lock()?;
    let Some(live) = self.live() else { return Ok(()) };
    let Err(stale) = self.compared(&live) else { return Ok(()) };
    let words = &LANES.placed;
    let binary = self.testbed_in(&live.dir).ok_or_else(|| {
      fill_marks(
        &words.live_uncarried,
        &[("release", &live.dir.to_string_lossy()), ("binary", &self.testbed_name())],
      )
    });

    match binary.and_then(|binary| self.copied(&binary, &live.commit)) {
      Ok(_) => {
        eprintln!(
          "{}",
          fill_marks(
            &words.brought_live,
            &[("machine", self.settings.label()), ("live", &short(&live.commit))]
          )
        );
        Ok(())
      }
      Err(why) => Err(fill_marks(&words.unbrought, &[("stale", &stale), ("said", &why)])),
    }
  }

  /// Whether the machine answers; what `ssh` said where it does not. Where it does not, an outage of it begins
  /// unless one stands; where it does, an outage standing ends, and the one process that ends an outage that was
  /// told sends `placed-recovered`.
  pub fn answers(&self) -> Result<(), String> {
    let probed = self.ssh().arg("true").output();
    let outage = Outage::of(&self.data_root, self.settings.label());

    if !succeeded(&probed) {
      outage.begun();
      return Err(said(&probed));
    }

    if let Some(since) = outage.ended() {
      let machine = self.settings.label();

      self.hook.send(&Event {
        name: "placed-recovered",
        commit: None,
        summary: fill_marks(&LANES.placed.recovered_event, &[("machine", machine), ("since", &since)]),
        detail: json!({ "machine": machine, "since": since }),
      });
    }

    Ok(())
  }

  /// A run's way onto the machine (docs/boundary.md, 「The placed machine」): the machine answering and its testbed
  /// the live release, asked again at the wait's pause while the machine does not answer. The code the run ends with
  /// where it does not come to that, its line said and its event sent: the wait run out, or the testbed there not
  /// brought to the live release while the machine answers. Nothing runs here in either case.
  pub fn waited_ready(&self, ask: &PlaceAsk<'_>) -> Result<(), u8> {
    let started = Instant::now();

    loop {
      let unanswered = match self.answers() {
        Ok(()) => match self.brought_live() {
          Ok(()) => return Ok(()),
          Err(why) => match self.answers() {
            Ok(()) => return Err(self.release_failed(ask, &why)),
            Err(said) => said,
          },
        },
        Err(said) => said,
      };
      let waited = started.elapsed();
      let bound = self.unreached.bound;

      if waited >= bound {
        return Err(self.gave_up(ask, waited, &unanswered));
      }

      eprintln!(
        "{}",
        fill_marks(
          &LANES.placed.unreached_waiting,
          &[
            ("machine", self.settings.label()),
            ("seconds", &waited.as_secs().to_string()),
            ("bound", &bound.as_secs().to_string()),
            ("said", &unanswered),
          ],
        )
      );
      std::thread::sleep(self.unreached.pause.min(bound.saturating_sub(waited)));
    }
  }

  // A run whose machine did not answer within the wait: its line, the event of the outage where no run told it
  // before, and the code of a run not run.
  fn gave_up(&self, ask: &PlaceAsk<'_>, waited: Duration, unanswered: &str) -> u8 {
    let machine = self.settings.label();
    let seconds = waited.as_secs().to_string();
    let outage = Outage::of(&self.data_root, machine);

    println!(
      "{}",
      fill_marks(
        &LANES.placed.unreached_gave_up,
        &[("machine", machine), ("seconds", &seconds), ("said", unanswered)],
      )
    );

    if outage.first_to_tell() {
      self.hook.send(&Event {
        name: "placed-unreachable",
        commit: None,
        summary: fill_marks(&LANES.placed.unreachable_event, &[("machine", machine)]),
        detail: json!({
          "machine": machine,
          "since": outage.since(),
          "command": ask.command,
          "waited_seconds": waited.as_secs(),
          "said": unanswered,
        }),
      });
    }

    NOT_RUN_EXIT_CODE
  }

  // A run whose machine answers but whose testbed could not be brought to the live release: its line, its event,
  // and the code of a run not run.
  fn release_failed(&self, ask: &PlaceAsk<'_>, why: &str) -> u8 {
    let machine = self.settings.label();
    let live = self.live().map(|live| live.commit);

    println!("{}", fill_marks(&LANES.placed.release_failed, &[("said", why)]));
    self.hook.send(&Event {
      name: "placed-release-failed",
      commit: None,
      summary: fill_marks(
        &LANES.placed.release_failed_event,
        &[("machine", machine), ("live", live.as_deref().map(short).unwrap_or_default().as_str())],
      ),
      detail: json!({
        "machine": machine,
        "command": ask.command,
        "live": live,
        "far": self.far_built_from().ok(),
        "said": why,
      }),
    });

    NOT_RUN_EXIT_CODE
  }

  /// The machine as `status` reads it: asked over `ssh` for the `built-from` beside its testbed, the connection
  /// given the policy's seconds to come up, and compared with the live testbed release.
  pub fn state(&self) -> MachineState {
    let (program, options) = self.ssh_program();
    let read = std::process::Command::new(program)
      .args(options)
      .args(["-o", &format!("ConnectTimeout={}", LANES.numbers.status_connect_seconds)])
      .arg(&self.settings.destination)
      .arg(format!("cat {}", far_path(&self.far_built_from_file())))
      .stdin(std::process::Stdio::null())
      .output();
    // `ssh` ends with 255 of its own where it could not reach the machine; any other end is the machine's answer.
    let reached = read.as_ref().is_ok_and(|done| done.status.code() != Some(255));
    let release = read
      .as_ref()
      .ok()
      .filter(|done| done.status.success())
      .map(|done| String::from_utf8_lossy(&done.stdout).trim().to_owned())
      .filter(|commit| !commit.is_empty());
    let live = self.live().map(|live| live.commit);

    MachineState {
      machine: self.settings.label().to_owned(),
      reached,
      said: (!reached).then(|| said(&read)),
      outage_since: Outage::of(&self.data_root, self.settings.label()).since(),
      current: release.is_some() && release == live,
      release,
      live,
    }
  }
}
