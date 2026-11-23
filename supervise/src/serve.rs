//! `supervise serve` (docs/supervise.md, 「Serving」): the refusals first — a systemd user manager that answers,
//! another supervisor on the same data root — then the stand-ins laid, the socket bound, and the supervisor run
//! until a SIGTERM, a SIGINT or `stop-all` has stopped every unit.

use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::supervise::SUPERVISE;
use tokio::net::UnixListener;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::oneshot;

use crate::control::Request;
use crate::manager::{Event, Supervisor, serve_socket};
use crate::places::Places;
use crate::unit::Specifiers;

/// The variable naming the runtime directory, which the supervisor hands on as a user manager does
/// (docs/supervise.md, 「How it runs a unit」).
pub const RUNTIME_VARIABLE: &str = "XDG_RUNTIME_DIR";

/// The variables naming the account, which the supervisor hands on as a user manager does.
pub const ACCOUNT_VARIABLES: [&str; 2] = ["USER", "LOGNAME"];

/// Why the supervisor would not serve.
#[derive(Debug)]
pub enum Refused {
  /// A systemd user manager answers: it runs the set.
  SystemdStands,
  /// Another supervisor holds the data root.
  Held(PathBuf),
  /// The socket or the folder could not be made.
  Unready(String),
}

impl Refused {
  pub fn said(&self) -> String {
    let said = &SUPERVISE.said;

    match self {
      Refused::SystemdStands => said.systemd_stands.clone(),
      Refused::Held(dir) => fill_marks(&said.held, &[("dir", &dir.to_string_lossy())]),
      Refused::Unready(why) => why.clone(),
    }
  }
}

/// Whether a systemd user manager serves this account: its private socket stands in the runtime directory.
pub fn systemd_user_manager_stands(runtime: &Path) -> bool {
  runtime.join("systemd/private").exists()
}

/// The stand-ins laid in the supervisor's folder: a link of each name to this binary, which answers by the name it
/// is called by.
pub fn lay_stand_ins(places: &Places, binary: &Path) -> std::io::Result<()> {
  let folder = places.stand_ins();

  std::fs::create_dir_all(&folder)?;

  for name in &SUPERVISE.places.names {
    let link = folder.join(name);

    if std::fs::read_link(&link).ok().as_deref() != Some(binary) {
      let _ = std::fs::remove_file(&link);
      std::os::unix::fs::symlink(binary, &link)?;
    }
  }

  Ok(())
}

// The data root held for this supervisor alone, for as long as the file stays open.
fn hold(places: &Places) -> Result<std::fs::File, Refused> {
  std::fs::create_dir_all(&places.folder).map_err(|why| Refused::Unready(why.to_string()))?;

  let file = std::fs::OpenOptions::new()
    .create(true)
    .truncate(false)
    .write(true)
    .open(places.lock())
    .map_err(|why| Refused::Unready(why.to_string()))?;

  // SAFETY: flock only takes an advisory lock on a descriptor this process holds open.
  let held = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };

  if held != 0 {
    return Err(Refused::Held(places.folder.clone()));
  }

  Ok(file)
}

/// Serves the units of `places` until the supervisor is stopped; `ready` is told once the socket listens.
pub async fn serve(
  places: Places,
  specifiers: Specifiers,
  binary: &Path,
  anyway: bool,
  ready: Option<oneshot::Sender<()>>,
  say: &(dyn Fn(&str) + Sync),
) -> Result<(), Refused> {
  if !anyway && systemd_user_manager_stands(&specifiers.runtime) {
    return Err(Refused::SystemdStands);
  }

  let _held = hold(&places)?;

  lay_stand_ins(&places, binary).map_err(|why| Refused::Unready(why.to_string()))?;

  let socket = places.socket();
  let _ = std::fs::remove_file(&socket);
  let listener = UnixListener::bind(&socket).map_err(|why| {
    Refused::Unready(fill_marks(
      &SUPERVISE.said.no_socket,
      &[("socket", &socket.to_string_lossy()), ("why", &why.to_string())],
    ))
  })?;
  let runtime = places.runtime(&specifiers.runtime).map_err(|why| Refused::Unready(why.to_string()))?;
  let mut env: Vec<(String, String)> =
    std::env::vars().filter(|(key, _)| key != RUNTIME_VARIABLE).collect();

  env.push((RUNTIME_VARIABLE.to_owned(), runtime.to_string_lossy().into_owned()));

  // The account's name, as a user manager hands it to every unit, where the supervisor's own environment lacks it.
  for name in ACCOUNT_VARIABLES {
    if !env.iter().any(|(key, value)| key == name && !value.is_empty()) {
      env.retain(|(key, _)| key != name);
      env.push((name.to_owned(), specifiers.user.clone()));
    }
  }

  let specifiers = Specifiers {
    runtime,
    ..specifiers
  };
  let (supervisor, received) = Supervisor::new(places.clone(), specifiers, env);
  let events = supervisor.sender();

  say(&fill_marks(
    &SUPERVISE.said.started,
    &[
      ("units", &places.unit_names().len().to_string()),
      ("dir", &places.unit_dir.to_string_lossy()),
      ("socket", &socket.to_string_lossy()),
    ],
  ));

  let socket_task = tokio::spawn(serve_socket(listener, events.clone()));
  let stopping = events.clone();
  let signals = tokio::spawn(async move {
    let (Ok(mut terminate), Ok(mut interrupt)) =
      (signal(SignalKind::terminate()), signal(SignalKind::interrupt()))
    else {
      return;
    };

    tokio::select! {
      _ = terminate.recv() => {}
      _ = interrupt.recv() => {}
    }

    let (reply, _) = oneshot::channel();
    let _ = stopping.send(Event::Request(Request::Shutdown, reply));
  });

  if let Some(ready) = ready {
    let _ = ready.send(());
  }

  supervisor.run(received).await;
  say(&SUPERVISE.said.ended);
  socket_task.abort();
  signals.abort();
  let _ = std::fs::remove_file(&socket);
  Ok(())
}
