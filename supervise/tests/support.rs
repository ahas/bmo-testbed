//! What the supervisor's tests share: a unit directory and a data root in a temporary folder, unit files written
//! into it, and a supervisor run over it in this process.

use std::path::{Path, PathBuf};
use std::time::Duration;

use bmo_testbed_supervise::control::{Answer, Request};
use bmo_testbed_supervise::manager::{Event, Supervisor};
use bmo_testbed_supervise::places::Places;
use bmo_testbed_supervise::state::UnitState;
use bmo_testbed_supervise::unit::Specifiers;
use tokio::sync::{mpsc, oneshot};

pub struct Bench {
  pub dir: tempfile::TempDir,
  pub places: Places,
}

impl Bench {
  pub fn new() -> Self {
    let dir = tempfile::tempdir().unwrap();
    let places =
      Places::new(&dir.path().join("data"), Some(&dir.path().join("config").to_string_lossy()), dir.path());

    std::fs::create_dir_all(&places.unit_dir).unwrap();
    Self { dir, places }
  }

  pub fn path(&self, name: &str) -> PathBuf {
    self.dir.path().join(name)
  }

  /// A unit file written into the unit directory, enabled where asked.
  pub fn unit(&self, name: &str, text: &str, enabled: bool) {
    std::fs::write(self.places.unit_dir.join(name), text).unwrap();

    if enabled {
      self.places.enable(name).unwrap();
    }
  }

  pub fn specifiers(&self) -> Specifiers {
    Specifiers {
      home: self.dir.path().to_path_buf(),
      user: "someone".to_owned(),
      uid: 1000,
      runtime: self.path("runtime"),
    }
  }

  /// A supervisor run over the bench; the sender its events go through.
  pub fn run(&self) -> mpsc::UnboundedSender<Event> {
    let env = vec![
      ("PATH".to_owned(), std::env::var("PATH").unwrap_or_default()),
      ("HOME".to_owned(), self.dir.path().to_string_lossy().into_owned()),
    ];
    let (supervisor, received) = Supervisor::new(self.places.clone(), self.specifiers(), env);
    let events = supervisor.sender();

    tokio::spawn(supervisor.run(received));
    events
  }
}

/// A request asked of a supervisor and its answer.
pub async fn ask(events: &mpsc::UnboundedSender<Event>, request: Request) -> Answer {
  let (reply, answered) = oneshot::channel();

  events.send(Event::Request(request, reply)).unwrap();
  tokio::time::timeout(Duration::from_secs(20), answered)
    .await
    .expect("the supervisor answers")
    .unwrap()
}

pub async fn state(events: &mpsc::UnboundedSender<Event>, unit: &str) -> UnitState {
  ask(
    events,
    Request::Show {
      units: vec![unit.to_owned()],
    },
  )
  .await
  .states
  .remove(0)
}

/// Waits until the unit's state meets `wanted`, up to a bound, and hands the state back.
pub async fn until(
  events: &mpsc::UnboundedSender<Event>,
  unit: &str,
  wanted: impl Fn(&UnitState) -> bool,
) -> UnitState {
  for _ in 0..200 {
    let now = state(events, unit).await;

    if wanted(&now) {
      return now;
    }

    tokio::time::sleep(Duration::from_millis(50)).await;
  }

  panic!("{unit} never reached the state asked: {:?}", state(events, unit).await);
}

/// Whether a process of this id still runs.
pub fn alive(pid: u32) -> bool {
  // SAFETY: a zero signal only asks whether the process stands.
  unsafe { libc::kill(pid as i32, 0) == 0 }
}

pub fn units(names: &[&str]) -> Vec<String> {
  names.iter().map(|name| (*name).to_owned()).collect()
}

pub fn read(path: &Path) -> String {
  std::fs::read_to_string(path).unwrap_or_default()
}
