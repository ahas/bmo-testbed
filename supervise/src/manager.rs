//! The supervisor itself (docs/supervise.md): one task that holds every unit's state and acts on one event at a
//! time — a request from the control socket, a process that ended, a pause before a restart that ran out, a stop
//! that passed its bound, a timer's elapse, a health answer. Each process runs in a process group of its own,
//! which stands in for the control group systemd would give it, and writes its output into its unit's log.

use std::collections::{BTreeMap, VecDeque};
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use bmo_testbed_core::declaration::Declaration;
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::supervise::SUPERVISE;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;

use crate::control::{Answer, Request};
use crate::env_file;
use crate::places::Places;
use crate::state::{Health, Snapshot, UnitState};
use crate::unit::{self, Ending, KillMode, Kind, Line, Service, ServiceType, Specifiers, Unit};

/// The code systemctl ends with where a unit is not found.
pub const NOT_FOUND_CODE: i32 = 5;

/// What the supervisor acts on, one at a time.
#[derive(Debug)]
pub enum Event {
  Request(Request, oneshot::Sender<Answer>),
  Exited {
    unit: String,
    generation: u64,
    code: Option<i32>,
    signal: Option<i32>,
    core: bool,
  },
  RestartDue {
    unit: String,
    generation: u64,
  },
  StopBound {
    unit: String,
    generation: u64,
  },
  TimerDue {
    timer: String,
    generation: u64,
  },
  HealthDue,
  Health {
    unit: String,
    generation: u64,
    health: Health,
  },
}

// Where a unit's process stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
  // The `ExecStartPre=` line of this index runs.
  Pre(usize),
  Main,
}

#[derive(Debug, Clone, Copy)]
struct Running {
  pid: u32,
  phase: Phase,
}

// One unit and what is under way for it.
struct Slot {
  unit: Option<Unit>,
  transient: bool,
  collect: bool,
  state: UnitState,
  running: Option<Running>,
  generation: u64,
  stopping: bool,
  // A start owed once the stop under way has ended: a restart.
  then_start: bool,
  starts: VecDeque<Instant>,
  // Answered once the stop under way has ended.
  on_stopped: Vec<oneshot::Sender<Answer>>,
  // Answered once the start under way is settled: the main process runs, or a oneshot's has ended.
  on_started: Vec<oneshot::Sender<Answer>>,
  // Answered once the unit's process has ended, with its code: `systemd-run --wait`.
  on_ended: Vec<oneshot::Sender<Answer>>,
}

impl Slot {
  fn new(unit: Option<Unit>, state: UnitState) -> Self {
    Self {
      unit,
      transient: false,
      collect: false,
      state,
      running: None,
      generation: 0,
      stopping: false,
      then_start: false,
      starts: VecDeque::new(),
      on_stopped: Vec::new(),
      on_started: Vec::new(),
      on_ended: Vec::new(),
    }
  }

  fn service(&self) -> Option<&Service> {
    match self.unit.as_ref().map(|unit| &unit.kind) {
      Some(Kind::Service(service)) => Some(service),
      _ => None,
    }
  }
}

/// The supervisor over one unit directory and one data root.
pub struct Supervisor {
  places: Places,
  specifiers: Specifiers,
  // The environment every unit starts from: the supervisor's own, the stand-ins first on its PATH and the
  // socket named.
  base_env: Vec<(String, String)>,
  slots: BTreeMap<String, Slot>,
  fired: BTreeMap<String, i64>,
  timer_generations: BTreeMap<String, u64>,
  events: mpsc::UnboundedSender<Event>,
  began: Instant,
  transient_count: u64,
  shutting_down: Option<Vec<oneshot::Sender<Answer>>>,
}

impl Supervisor {
  /// A supervisor over `places`, starting units from the environment `env`; the receiving end of its events.
  pub fn new(
    places: Places,
    specifiers: Specifiers,
    env: Vec<(String, String)>,
  ) -> (Self, mpsc::UnboundedReceiver<Event>) {
    let (events, received) = mpsc::unbounded_channel();
    let stand_ins = places.stand_ins();
    let mut base_env: Vec<(String, String)> =
      env.into_iter().filter(|(key, _)| key != &SUPERVISE.places.variable).collect();
    let path = base_env
      .iter()
      .find(|(key, _)| key == "PATH")
      .map(|(_, value)| value.clone())
      .unwrap_or_default();

    base_env.retain(|(key, _)| key != "PATH");
    base_env.push(("PATH".to_owned(), with_stand_ins(&stand_ins, &path)));
    base_env.push((SUPERVISE.places.variable.clone(), places.socket().to_string_lossy().into_owned()));

    let fired = read_snapshot(&places.state())
      .map(|snapshot| snapshot.fired.into_iter().collect())
      .unwrap_or_default();
    let supervisor = Self {
      places,
      specifiers,
      base_env,
      slots: BTreeMap::new(),
      fired,
      timer_generations: BTreeMap::new(),
      events,
      began: Instant::now(),
      transient_count: 0,
      shutting_down: None,
    };

    (supervisor, received)
  }

  /// The sender a control socket and a signal handler hand their events through.
  pub fn sender(&self) -> mpsc::UnboundedSender<Event> {
    self.events.clone()
  }

  /// Reads every unit, starts the enabled timers and services in their order, and acts on events until a
  /// shutdown has stopped every unit.
  pub async fn run(mut self, mut received: mpsc::UnboundedReceiver<Event>) {
    std::fs::create_dir_all(self.places.logs()).ok();
    self.reload();

    for name in self.boot_order() {
      let (sender, _) = oneshot::channel();
      self.start(&name, sender);
    }

    self.arm_timers();

    let health_events = self.events.clone();
    let every = Duration::from_secs(SUPERVISE.numbers.health_every_seconds.max(1));

    tokio::spawn(async move {
      let mut ticks = tokio::time::interval(every);

      loop {
        ticks.tick().await;

        if health_events.send(Event::HealthDue).is_err() {
          return;
        }
      }
    });

    self.write_state();

    while let Some(event) = received.recv().await {
      self.handle(event);
      self.write_state();

      if self.shut_down() {
        break;
      }
    }

    let mut snapshot = self.snapshot();

    snapshot.pid = 0;
    write_snapshot(&self.places.state(), &snapshot);

    // The shutdown is answered once the state file says the supervisor has ended.
    for waiter in self.shutting_down.take().unwrap_or_default() {
      let _ = waiter.send(Answer::said(SUPERVISE.said.ended.clone()));
    }
  }

  fn handle(&mut self, event: Event) {
    match event {
      Event::Request(request, reply) => self.request(request, reply),
      Event::Exited {
        unit,
        generation,
        code,
        signal,
        core,
      } => self.exited(&unit, generation, code, signal, core),
      Event::RestartDue { unit, generation } => {
        let due = self.slots.get(&unit).is_some_and(|slot| {
          slot.generation == generation && slot.running.is_none() && slot.state.sub_state == "auto-restart"
        });

        if due {
          if let Some(slot) = self.slots.get_mut(&unit) {
            slot.state.n_restarts += 1;
          }

          let (sender, _) = oneshot::channel();
          self.start(&unit, sender);
        }
      }
      Event::StopBound { unit, generation } => {
        if let Some(slot) =
          self.slots.get(&unit).filter(|slot| slot.generation == generation && slot.stopping)
          && let Some(running) = slot.running
        {
          let mode = slot.service().map_or(KillMode::Group, |service| service.kill_mode);

          signal_process(running.pid, mode != KillMode::Process, libc::SIGKILL);
        }
      }
      Event::TimerDue { timer, generation } => {
        if self.timer_generations.get(&timer) == Some(&generation) {
          self.fire(&timer);
        }
      }
      Event::HealthDue => self.ask_health(),
      Event::Health {
        unit,
        generation,
        health,
      } => {
        if let Some(slot) = self.slots.get_mut(&unit).filter(|slot| slot.generation == generation) {
          slot.state.health = if slot.state.is_active() { health } else { Health::Unknown };
        }
      }
    }
  }

  fn request(&mut self, request: Request, reply: oneshot::Sender<Answer>) {
    match request {
      Request::Start { units } => self.each(units, reply, |me, unit, sender| me.start(unit, sender)),
      Request::Stop { units } => self.each(units, reply, |me, unit, sender| me.stop(unit, sender)),
      Request::Restart { units } => self.each(units, reply, |me, unit, sender| me.restart(unit, sender)),
      Request::TryRestart { units } => self.each(units, reply, |me, unit, sender| {
        if me.slots.get(unit).is_some_and(|slot| slot.state.is_active()) {
          me.restart(unit, sender);
        } else {
          let _ = sender.send(Answer::ok());
        }
      }),
      Request::ResetFailed { units } => {
        let named = if units.is_empty() { self.slots.keys().cloned().collect() } else { units };

        for name in named {
          if let Some(slot) = self.slots.get_mut(&name) {
            slot.starts.clear();

            if slot.state.active_state == "failed" {
              slot.state.set("inactive", "dead");
              slot.state.result = "success".to_owned();
            }
          }
        }

        let _ = reply.send(Answer::ok());
      }
      Request::DaemonReload => {
        self.reload();
        self.arm_timers();
        let _ = reply.send(Answer::ok());
      }
      Request::Enable { units, now } => {
        let mut answer = Answer::ok();
        let mut names = Vec::new();

        for named in units {
          let name = if named.contains('/') {
            match self.places.link_file(Path::new(&named)) {
              Ok(name) => name,
              Err(why) => {
                answer = answer.and(Answer::failed(1, format!("{named}: {why}")));
                continue;
              }
            }
          } else {
            named
          };

          match self.places.enable(&name) {
            Ok(Some((link, target))) => {
              answer = answer.and(Answer::said(fill_marks(
                &SUPERVISE.said.enable_link,
                &[("link", &link.to_string_lossy()), ("target", &target.to_string_lossy())],
              )));
            }
            Ok(None) => {}
            Err(why) => answer = answer.and(Answer::failed(1, format!("{name}: {why}"))),
          }

          names.push(name);
        }

        self.reload();
        self.arm_timers();

        if now {
          self.each_then(names, answer, reply, |me, unit, sender| me.start(unit, sender));
        } else {
          let _ = reply.send(answer);
        }
      }
      Request::Disable { units, now } => {
        let mut answer = Answer::ok();

        for name in &units {
          match self.places.disable(name) {
            Ok(Some(link)) => {
              answer = answer.and(Answer::said(fill_marks(
                &SUPERVISE.said.disable_link,
                &[("link", &link.to_string_lossy())],
              )));
            }
            Ok(None) => {}
            Err(why) => answer = answer.and(Answer::failed(1, format!("{name}: {why}"))),
          }
        }

        self.reload();
        self.arm_timers();

        if now {
          self.each_then(units, answer, reply, |me, unit, sender| me.stop(unit, sender));
        } else {
          let _ = reply.send(answer);
        }
      }
      Request::Kill { units, signal } => {
        for name in &units {
          if let Some(slot) = self.slots.get(name)
            && let Some(running) = slot.running
          {
            let mode = slot.service().map_or(KillMode::Group, |service| service.kill_mode);

            signal_process(running.pid, mode != KillMode::Process, signal);
          }
        }

        let _ = reply.send(Answer::ok());
      }
      Request::Show { units } => {
        let states = if units.is_empty() {
          self.slots.values().map(|slot| slot.state.clone()).collect()
        } else {
          units
            .iter()
            .map(|name| {
              self
                .slots
                .get(name)
                .map_or_else(|| UnitState::not_found(name), |slot| slot.state.clone())
            })
            .collect()
        };

        let _ = reply.send(Answer {
          states,
          ..Answer::ok()
        });
      }
      Request::ShowEnvironment => {
        let out = self
          .base_env
          .iter()
          .map(|(key, value)| format!("{key}={value}"))
          .collect::<Vec<_>>()
          .join("\n");

        let _ = reply.send(Answer::said(out));
      }
      Request::Run {
        unit,
        words,
        cwd,
        env,
        collect,
        wait,
      } => self.run_transient(unit, words, cwd, env, collect, wait, reply),
      Request::Shutdown => {
        let names: Vec<String> = self.boot_order().into_iter().rev().collect();
        let others: Vec<String> = self.slots.keys().filter(|name| !names.contains(name)).cloned().collect();

        for name in names.into_iter().chain(others) {
          let (sender, _) = oneshot::channel();
          self.stop(&name, sender);
        }

        self.timer_generations.clear();
        self.shutting_down.get_or_insert_with(Vec::new).push(reply);
      }
    }
  }

  // Acts on each named unit, answering once all of them have answered, their answers joined.
  fn each(
    &mut self,
    units: Vec<String>,
    reply: oneshot::Sender<Answer>,
    act: impl FnMut(&mut Self, &str, oneshot::Sender<Answer>),
  ) {
    self.each_then(units, Answer::ok(), reply, act);
  }

  fn each_then(
    &mut self,
    units: Vec<String>,
    first: Answer,
    reply: oneshot::Sender<Answer>,
    mut act: impl FnMut(&mut Self, &str, oneshot::Sender<Answer>),
  ) {
    let mut waits = Vec::new();

    for name in &units {
      let (sender, received) = oneshot::channel();

      if self.slots.contains_key(name) {
        act(self, name, sender);
      } else {
        let _ = sender
          .send(Answer::failed(NOT_FOUND_CODE, fill_marks(&SUPERVISE.said.not_found, &[("unit", name)])));
      }

      waits.push(received);
    }

    tokio::spawn(async move {
      let mut answer = first;

      for wait in waits {
        answer = answer.and(wait.await.unwrap_or_default());
      }

      let _ = reply.send(answer);
    });
  }

  /// Every unit file of the unit directory read again; a running process keeps the file it started from until
  /// its next start, and a unit gone from the directory is let go once it runs no more.
  fn reload(&mut self) {
    let names = self.places.unit_names();

    for name in &names {
      let enabled = self.places.enabled(name);
      let read = std::fs::read_to_string(self.places.unit_dir.join(name))
        .map_err(|why| why.to_string())
        .and_then(|text| unit::read(name, &text, &self.specifiers));
      let slot = self
        .slots
        .entry(name.clone())
        .or_insert_with(|| Slot::new(None, UnitState::loaded(name, "", enabled)));

      slot.transient = false;
      slot.state.unit_file_state = if enabled { "enabled" } else { "disabled" }.to_owned();

      match read {
        Ok(unit) => {
          slot.state.load_state = "loaded".to_owned();
          slot.state.description = unit.description.clone();
          slot.state.mark = unit.mark.clone().unwrap_or_default();
          slot.state.not_applied = unit.not_applied.clone();
          slot.unit = Some(unit);
        }
        Err(why) => {
          slot.state.load_state = "error".to_owned();
          slot.state.description = fill_marks(&SUPERVISE.said.unreadable, &[("unit", name), ("why", &why)]);
          slot.unit = None;
        }
      }
    }

    self
      .slots
      .retain(|name, slot| names.contains(name) || slot.transient || slot.running.is_some());
  }

  // The enabled services in the order they start: each after the units its `After=` names among them.
  fn boot_order(&self) -> Vec<String> {
    let enabled: Vec<&String> = self
      .slots
      .iter()
      .filter(|(name, slot)| name.ends_with(".service") && !slot.transient && self.places.enabled(name))
      .map(|(name, _)| name)
      .collect();
    let mut order: Vec<String> = Vec::new();
    let mut left: Vec<&String> = enabled.clone();

    while !left.is_empty() {
      let ready = left.iter().position(|name| {
        let after = self.slots[*name].unit.as_ref().map(|unit| unit.after.clone()).unwrap_or_default();

        after.iter().all(|before| order.contains(before) || !enabled.contains(&before))
      });

      // A ring of `After=` starts in name order.
      let at = ready.unwrap_or(0);

      order.push(left.remove(at).clone());
    }

    order
  }

  fn start(&mut self, name: &str, reply: oneshot::Sender<Answer>) {
    let Some(slot) = self.slots.get_mut(name) else {
      let _ = reply
        .send(Answer::failed(NOT_FOUND_CODE, fill_marks(&SUPERVISE.said.not_found, &[("unit", name)])));
      return;
    };

    if name.ends_with(".timer") {
      slot.state.set("active", "waiting");
      slot.state.active_since_ms = now_ms();
      let _ = reply.send(Answer::ok());
      self.arm_timers();
      return;
    }

    if slot.running.is_some() || (slot.state.active_state == "active" && slot.stopping) {
      if slot.stopping {
        slot.then_start = true;
        slot.on_started.push(reply);
      } else if slot.state.sub_state == "start"
        || matches!(slot.running.map(|running| running.phase), Some(Phase::Pre(_)))
      {
        slot.on_started.push(reply);
      } else {
        let _ = reply.send(Answer::ok());
      }

      return;
    }

    if slot.state.active_state == "active" {
      let _ = reply.send(Answer::ok());
      return;
    }

    let Some(unit) = slot.unit.clone() else {
      let _ = reply.send(Answer::failed(
        1,
        fill_marks(&SUPERVISE.said.start_failed, &[("unit", name), ("why", &slot.state.description)]),
      ));
      return;
    };
    let Kind::Service(service) = &unit.kind else {
      let _ = reply.send(Answer::ok());
      return;
    };

    if !unit::conditions_hold(&unit) {
      slot.state.set("inactive", "dead");
      let _ = reply.send(Answer::ok());
      return;
    }

    let now = Instant::now();

    while slot
      .starts
      .front()
      .is_some_and(|at| now.duration_since(*at) > service.start_limit_interval)
    {
      slot.starts.pop_front();
    }

    if slot.starts.len() as u32 >= service.start_limit_burst {
      slot.state.set("failed", "failed");
      slot.state.result = "start-limit-hit".to_owned();
      let _ = reply.send(Answer::failed(1, fill_marks(&SUPERVISE.said.start_limit, &[("unit", name)])));
      return;
    }

    slot.starts.push_back(now);
    slot.on_started.push(reply);
    slot.state.result = "success".to_owned();

    let first = if service.start_pre.is_empty() { Phase::Main } else { Phase::Pre(0) };

    self.spawn_phase(name, first);
  }

  // Starts the process of a phase of the unit's start; a process that cannot start fails the unit.
  fn spawn_phase(&mut self, name: &str, phase: Phase) {
    let base_env = self.base_env.clone();
    let stand_ins = self.places.stand_ins();
    let home = self.specifiers.home.clone();
    let log = self.places.log_of(name);
    let events = self.events.clone();
    let Some(slot) = self.slots.get_mut(name) else { return };
    let Some(service) = slot.service().cloned() else { return };
    let line = match phase {
      Phase::Pre(index) => service.start_pre.get(index).cloned(),
      Phase::Main => service.start.clone(),
    };
    let Some(line) = line else {
      fail(slot, SUPERVISE.said.no_command.clone(), "resources");
      return;
    };
    let launched = launch(&line, &service, &base_env, &stand_ins, &home, &log);
    let child = match launched {
      Ok(child) => child,
      Err(why) => {
        fail(slot, why, "resources");
        return;
      }
    };

    slot.generation += 1;

    let generation = slot.generation;
    let pid = child.id().unwrap_or(0);

    slot.running = Some(Running { pid, phase });
    slot.state.main_pid = pid;

    match (phase, service.kind) {
      (Phase::Pre(_), _) => slot.state.set("activating", "start-pre"),
      (Phase::Main, ServiceType::Oneshot) => slot.state.set("activating", "start"),
      (Phase::Main, ServiceType::Simple) => {
        slot.state.set("active", "running");
        slot.state.active_since_ms = now_ms();

        for waiter in slot.on_started.drain(..) {
          let _ = waiter.send(Answer::ok());
        }
      }
    }

    watch(child, name.to_owned(), generation, log, events);
  }

  fn exited(&mut self, name: &str, generation: u64, code: Option<i32>, signal: Option<i32>, core: bool) {
    let Some(slot) = self.slots.get_mut(name) else { return };

    if slot.generation != generation {
      return;
    }

    let Some(running) = slot.running.take() else { return };
    let Some(service) = slot.service().cloned() else { return };
    let ending = service.ending(code, signal, core);

    slot.state.main_pid = 0;
    slot.state.exec_main_status = code.or(signal).unwrap_or(0);

    // What the main process left in its group goes with it, as systemd ends a unit's control group.
    if service.kill_mode != KillMode::Process {
      signal_process(running.pid, true, libc::SIGTERM);
    }

    if slot.stopping {
      slot.stopping = false;
      slot.state.set("inactive", "dead");

      for waiter in slot.on_stopped.drain(..) {
        let _ = waiter.send(Answer::ok());
      }

      for waiter in slot.on_ended.drain(..) {
        let _ = waiter.send(Answer::failed(code.unwrap_or(1), String::new()));
      }

      let owed = std::mem::take(&mut slot.then_start);

      if owed {
        let waiting = std::mem::take(&mut slot.on_started);

        for waiter in waiting {
          self.start(name, waiter);
        }
      } else {
        for waiter in slot.on_started.drain(..) {
          let _ = waiter.send(Answer::ok());
        }

        self.collect(name);
      }

      return;
    }

    if let Phase::Pre(index) = running.phase {
      if ending == Ending::Clean || service.start_pre[index].overlook_failure {
        let next = if index + 1 < service.start_pre.len() { Phase::Pre(index + 1) } else { Phase::Main };

        self.spawn_phase(name, next);
      } else {
        fail(
          slot,
          format!("ExecStartPre ended with {}", code.or(signal).unwrap_or(1)),
          result_of(ending),
        );
      }

      return;
    }

    let overlooked = service.start.as_ref().is_some_and(|line| line.overlook_failure);
    let clean = ending == Ending::Clean || overlooked;

    for waiter in slot.on_ended.drain(..) {
      let _ = waiter.send(Answer::failed(code.unwrap_or(1), String::new()));
    }

    if service.restart.restarts(if overlooked { Ending::Clean } else { ending }) {
      slot.state.set("activating", "auto-restart");
      slot.state.result = result_of(ending).to_owned();

      for waiter in slot.on_started.drain(..) {
        let _ = waiter.send(Answer::ok());
      }

      let (events, unit, after) = (self.events.clone(), name.to_owned(), service.restart_after);

      tokio::spawn(async move {
        tokio::time::sleep(after).await;
        let _ = events.send(Event::RestartDue { unit, generation });
      });

      return;
    }

    if clean {
      match (service.kind, service.remain_after_exit) {
        (ServiceType::Oneshot, true) => slot.state.set("active", "exited"),
        _ => slot.state.set("inactive", "dead"),
      }

      slot.state.result = "success".to_owned();

      for waiter in slot.on_started.drain(..) {
        let _ = waiter.send(Answer::ok());
      }
    } else {
      let why = format!("its process ended with {}", code.or(signal).unwrap_or(1));

      fail(slot, why, result_of(ending));
    }

    self.collect(name);
  }

  // A transient unit run with `--collect`, or one that ended well, let go of.
  fn collect(&mut self, name: &str) {
    let gone = self.slots.get(name).is_some_and(|slot| {
      slot.transient && slot.running.is_none() && (slot.collect || slot.state.active_state != "failed")
    });

    if gone {
      self.slots.remove(name);
    }
  }

  fn stop(&mut self, name: &str, reply: oneshot::Sender<Answer>) {
    let Some(slot) = self.slots.get_mut(name) else {
      let _ = reply
        .send(Answer::failed(NOT_FOUND_CODE, fill_marks(&SUPERVISE.said.not_found, &[("unit", name)])));
      return;
    };

    if name.ends_with(".timer") {
      slot.state.set("inactive", "dead");
      self.timer_generations.remove(name);
      let _ = reply.send(Answer::ok());
      return;
    }

    let Some(running) = slot.running else {
      // A pause before a restart is cut short; a unit that runs nothing is stopped already.
      slot.generation += 1;

      if slot.state.active_state != "failed" {
        slot.state.set("inactive", "dead");
      }

      slot.then_start = false;

      for waiter in slot.on_started.drain(..) {
        let _ = waiter.send(Answer::ok());
      }

      let _ = reply.send(Answer::ok());
      self.collect(name);
      return;
    };
    let (mode, signal, bound) = slot.service().map_or((KillMode::Group, libc::SIGTERM, None), |service| {
      (service.kill_mode, service.kill_signal, service.stop_bound)
    });

    slot.stopping = true;
    slot.state.set("deactivating", "stop-sigterm");
    slot.on_stopped.push(reply);
    signal_process(running.pid, mode == KillMode::Group, signal);

    if let Some(bound) = bound {
      let (events, unit, generation) = (self.events.clone(), name.to_owned(), slot.generation);

      tokio::spawn(async move {
        tokio::time::sleep(bound).await;
        let _ = events.send(Event::StopBound { unit, generation });
      });
    }
  }

  fn restart(&mut self, name: &str, reply: oneshot::Sender<Answer>) {
    let running = self.slots.get(name).is_some_and(|slot| slot.running.is_some());

    if running {
      let (stopped, _) = oneshot::channel();

      if let Some(slot) = self.slots.get_mut(name) {
        slot.then_start = true;
        slot.on_started.push(reply);
      }

      self.stop(name, stopped);
    } else {
      if let Some(slot) = self.slots.get_mut(name) {
        // A pause before a restart is cut short by the restart asked.
        slot.generation += 1;

        if slot.state.sub_state == "auto-restart" {
          slot.state.set("inactive", "dead");
        }
      }

      self.start(name, reply);
    }
  }

  #[allow(clippy::too_many_arguments)]
  fn run_transient(
    &mut self,
    unit: Option<String>,
    words: Vec<String>,
    cwd: Option<PathBuf>,
    env: Vec<(String, String)>,
    collect: bool,
    wait: bool,
    reply: oneshot::Sender<Answer>,
  ) {
    let name = match unit {
      Some(name) if name.contains('.') => name,
      Some(name) => format!("{name}.service"),
      None => {
        self.transient_count += 1;
        format!("run-p{}-i{}.service", std::process::id(), self.transient_count)
      }
    };

    if self.slots.get(&name).is_some_and(|slot| !slot.transient || slot.running.is_some()) {
      let _ =
        reply.send(Answer::failed(1, fill_marks(&SUPERVISE.said.transient_exists, &[("unit", &name)])));
      return;
    }

    let service = Service::transient(words, cwd, env);
    let unit = Unit {
      name: name.clone(),
      description: String::new(),
      after: Vec::new(),
      conditions: Vec::new(),
      mark: None,
      not_applied: Vec::new(),
      kind: Kind::Service(service),
    };
    let mut state = UnitState::loaded(&name, "", false);

    state.unit_file_state = "transient".to_owned();

    let mut slot = Slot::new(Some(unit), state);

    slot.transient = true;
    slot.collect = collect;
    self.slots.insert(name.clone(), slot);

    let (started, settled) = oneshot::channel();

    if wait {
      if let Some(slot) = self.slots.get_mut(&name) {
        slot.on_ended.push(reply);
      }

      self.start(&name, started);

      // A start that failed ends the wait at once.
      let failed = self.slots.get(&name).is_some_and(|slot| slot.running.is_none());

      if failed && let Some(slot) = self.slots.get_mut(&name) {
        let why = slot.state.description.clone();

        for waiter in slot.on_ended.drain(..) {
          let _ = waiter.send(Answer::failed(1, why.clone()));
        }
      }
    } else {
      self.start(&name, started);

      let said = format!("Running as unit: {name}");

      tokio::spawn(async move {
        let answer = settled.await.unwrap_or_default();

        let _ = reply.send(if answer.code == 0 { Answer::said(said) } else { answer });
      });
    }
  }

  // Each enabled timer that is active given its next elapse; a timer disabled or stopped is let go of.
  fn arm_timers(&mut self) {
    let timers: Vec<String> = self
      .slots
      .iter()
      .filter(|(name, slot)| {
        name.ends_with(".timer")
          && (self.places.enabled(name) || slot.state.active_state == "active")
          && self.shutting_down.is_none()
      })
      .map(|(name, _)| name.clone())
      .collect();

    self.timer_generations.retain(|name, _| timers.contains(name));

    for name in timers {
      let Some(Kind::Timer(timer)) =
        self.slots.get(&name).and_then(|slot| slot.unit.as_ref()).map(|unit| unit.kind.clone())
      else {
        continue;
      };
      let now = jiff::Timestamp::now();
      let zone = jiff::tz::TimeZone::system();
      let last = self.fired.get(&name).copied();
      let mut next: Option<jiff::Timestamp> = None;
      let mut earlier = |at: jiff::Timestamp| next = Some(next.map_or(at, |next| next.min(at)));

      for calendar in &timer.calendars {
        if let Ok(mut upcoming) = calendar.upcoming(now, &zone)
          && let Some(at) = upcoming.next()
        {
          earlier(at);
        }

        // A persistent timer that missed an elapse while the supervisor was down fires at once.
        if timer.persistent
          && let Some(last) = last.and_then(|last| jiff::Timestamp::from_millisecond(last).ok())
          && let Ok(mut since) = calendar.upcoming(last, &zone)
          && since.next().is_some_and(|missed| missed <= now)
        {
          earlier(now);
        }
      }

      if let Some(on_boot) = timer.on_boot {
        let at = self.began + on_boot;

        if at > Instant::now() {
          earlier(now + jiff::SignedDuration::try_from(at - Instant::now()).unwrap_or_default());
        }
      }

      if let (Some(span), Some(last)) =
        (timer.on_unit_active, last.and_then(|last| jiff::Timestamp::from_millisecond(last).ok()))
      {
        earlier((last + jiff::SignedDuration::try_from(span).unwrap_or_default()).max(now));
      }

      let Some(next) = next else { continue };
      let generation = self.timer_generations.get(&name).copied().unwrap_or(0) + 1;
      let delay =
        Duration::try_from(next.duration_since(now)).unwrap_or_default() + spread(timer.randomized_delay);
      let events = self.events.clone();
      let timer_name = name.clone();

      self.timer_generations.insert(name.clone(), generation);

      if let Some(slot) = self.slots.get_mut(&name) {
        slot.state.set("active", "waiting");
      }

      tokio::spawn(async move {
        tokio::time::sleep(delay).await;
        let _ = events.send(Event::TimerDue {
          timer: timer_name,
          generation,
        });
      });
    }
  }

  fn fire(&mut self, timer: &str) {
    let target = match self.slots.get(timer).and_then(|slot| slot.unit.as_ref()).map(|unit| &unit.kind) {
      Some(Kind::Timer(timer)) => timer.unit.clone(),
      _ => return,
    };

    self.fired.insert(timer.to_owned(), now_ms());
    self.timer_generations.remove(timer);

    let (sender, _) = oneshot::channel();

    self.start(&target, sender);
    self.arm_timers();
  }

  // The health of every running unit marked `bot` whose deploy tree declares one, asked in the tree.
  fn ask_health(&mut self) {
    let bound = Duration::from_secs(SUPERVISE.numbers.health_bound_seconds);

    for (name, slot) in &self.slots {
      if slot.state.mark != "bot" || slot.state.active_state != "active" {
        continue;
      }

      let Some(tree) = self.places.deploy_tree_of(name) else { continue };
      let Some(command) = declared_health(&tree) else { continue };
      let (events, unit, generation, env) =
        (self.events.clone(), name.clone(), slot.generation, self.base_env.clone());

      tokio::spawn(async move {
        let mut asked = tokio::process::Command::new("sh");

        asked
          .arg("-c")
          .arg(&command)
          .current_dir(&tree)
          .env_clear()
          .envs(env)
          .stdin(Stdio::null())
          .stdout(Stdio::null())
          .stderr(Stdio::null())
          .kill_on_drop(true);

        let health = match tokio::time::timeout(bound, async { asked.status().await }).await {
          Ok(Ok(status)) if status.success() => Health::Healthy,
          _ => Health::Unhealthy,
        };

        let _ = events.send(Event::Health {
          unit,
          generation,
          health,
        });
      });
    }
  }

  // Whether a shutdown asked has stopped every unit.
  fn shut_down(&self) -> bool {
    self.shutting_down.is_some() && self.slots.values().all(|slot| slot.running.is_none())
  }

  /// Every unit's state, and when each timer last fired.
  pub fn snapshot(&self) -> Snapshot {
    Snapshot {
      pid: std::process::id(),
      units: self.slots.values().map(|slot| slot.state.clone()).collect(),
      fired: self.fired.iter().map(|(name, at)| (name.clone(), *at)).collect(),
    }
  }

  fn write_state(&self) {
    write_snapshot(&self.places.state(), &self.snapshot());
  }
}

/// The control socket served: each connection's one request handed to the supervisor as an event, and its
/// answer written back.
pub async fn serve_socket(listener: UnixListener, events: mpsc::UnboundedSender<Event>) {
  loop {
    let Ok((stream, _)) = listener.accept().await else { continue };
    let events = events.clone();

    tokio::spawn(async move {
      let (read, mut write) = stream.into_split();
      let mut line = String::new();

      if BufReader::new(read).read_line(&mut line).await.is_err() {
        return;
      }

      let answer = match serde_json::from_str::<Request>(&line) {
        Ok(request) => {
          let (reply, answered) = oneshot::channel();

          if events.send(Event::Request(request, reply)).is_err() {
            return;
          }

          answered.await.unwrap_or_default()
        }
        Err(why) => Answer::failed(2, why.to_string()),
      };
      let mut out = serde_json::to_string(&answer).unwrap_or_default();

      out.push('\n');
      let _ = write.write_all(out.as_bytes()).await;
    });
  }
}

/// A PATH with the stand-ins' folder first, once.
pub fn with_stand_ins(stand_ins: &Path, path: &str) -> String {
  let first = stand_ins.to_string_lossy();
  let rest: Vec<&str> = path.split(':').filter(|part| !part.is_empty() && *part != first).collect();

  std::iter::once(first.as_ref()).chain(rest).collect::<Vec<_>>().join(":")
}

/// The environment a unit's process is started with: the supervisor's own, then each `Environment=`, then each
/// environment file over them, as systemd orders them, the stand-ins kept first on whatever PATH the unit names.
pub fn unit_environment(
  service: &Service,
  base: &[(String, String)],
  stand_ins: &Path,
) -> Result<Vec<(String, String)>, String> {
  let mut env: BTreeMap<String, String> = base.iter().cloned().collect();

  for (key, value) in &service.environment {
    env.insert(key.clone(), value.clone());
  }

  for (file, optional) in &service.environment_files {
    match std::fs::read_to_string(file) {
      Ok(text) => env.extend(env_file::assignments(&text)),
      Err(_) if *optional => {}
      Err(why) => return Err(format!("{}: {why}", file.display())),
    }
  }

  let path = env.get("PATH").cloned().unwrap_or_default();

  env.insert("PATH".to_owned(), with_stand_ins(stand_ins, &path));
  Ok(env.into_iter().collect())
}

fn launch(
  line: &Line,
  service: &Service,
  base_env: &[(String, String)],
  stand_ins: &Path,
  home: &Path,
  log: &Path,
) -> Result<tokio::process::Child, String> {
  let env = unit_environment(service, base_env, stand_ins)?;
  let dir = match &service.working_directory {
    Some((dir, _)) if dir.is_dir() => dir.clone(),
    Some((_, true)) | None => home.to_path_buf(),
    Some((dir, false)) => return Err(format!("the working directory {} does not stand", dir.display())),
  };
  let program = resolve_program(&line.words[0], &env);
  let mut command = tokio::process::Command::new(program);

  command
    .args(&line.words[1..])
    .current_dir(dir)
    .env_clear()
    .envs(env)
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .process_group(0)
    .kill_on_drop(false);

  command.spawn().map_err(|why| {
    let said = format!("{}: {why}", line.words[0]);

    append_line(log, &said);
    said
  })
}

// A program named without a folder is found on the PATH the unit is handed, not the supervisor's own.
fn resolve_program(program: &str, env: &[(String, String)]) -> PathBuf {
  if program.contains('/') {
    return PathBuf::from(program);
  }

  let path = env
    .iter()
    .find(|(key, _)| key == "PATH")
    .map(|(_, value)| value.as_str())
    .unwrap_or_default();

  path
    .split(':')
    .map(|dir| Path::new(dir).join(program))
    .find(|candidate| candidate.is_file())
    .unwrap_or_else(|| PathBuf::from(program))
}

// The child's output carried into its log line by line, and its end handed to the supervisor.
fn watch(
  mut child: tokio::process::Child,
  unit: String,
  generation: u64,
  log: PathBuf,
  events: mpsc::UnboundedSender<Event>,
) {
  let out = child.stdout.take();
  let err = child.stderr.take();

  tokio::spawn(async move {
    let carry_out = out.map(|out| tokio::spawn(carry(out, log.clone())));
    let carry_err = err.map(|err| tokio::spawn(carry(err, log.clone())));
    let status = child.wait().await;

    for carried in [carry_out, carry_err].into_iter().flatten() {
      let _ = tokio::time::timeout(Duration::from_secs(1), carried).await;
    }

    let (code, signal, core) = match status {
      Ok(status) => (status.code(), status.signal(), status.core_dumped()),
      Err(_) => (Some(1), None, false),
    };

    let _ = events.send(Event::Exited {
      unit,
      generation,
      code,
      signal,
      core,
    });
  });
}

async fn carry(stream: impl AsyncRead + Unpin, log: PathBuf) {
  let mut lines = BufReader::new(stream).lines();

  while let Ok(Some(line)) = lines.next_line().await {
    append_line(&log, &line);
  }
}

/// One line appended to a unit's log, stamped with the time it was written; a log past its cap is moved aside
/// first.
pub fn append_line(log: &Path, line: &str) {
  use std::io::Write as _;

  if std::fs::metadata(log).is_ok_and(|meta| meta.len() > SUPERVISE.numbers.log_cap_bytes) {
    let _ = std::fs::rename(log, log.with_extension("log.1"));
  }

  if let Some(dir) = log.parent() {
    let _ = std::fs::create_dir_all(dir);
  }

  if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(log) {
    let stamp = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ");
    let _ = writeln!(file, "{stamp} {line}");
  }
}

fn fail(slot: &mut Slot, why: String, result: &str) {
  slot.running = None;
  slot.state.main_pid = 0;
  slot.state.set("failed", "failed");
  slot.state.result = result.to_owned();

  let said = fill_marks(&SUPERVISE.said.start_failed, &[("unit", &slot.state.id), ("why", &why)]);

  for waiter in slot.on_started.drain(..).chain(slot.on_ended.drain(..)) {
    let _ = waiter.send(Answer::failed(1, said.clone()));
  }
}

fn result_of(ending: Ending) -> &'static str {
  match ending {
    Ending::Clean => "success",
    Ending::Code => "exit-code",
    Ending::Signal => "signal",
    Ending::Core => "core-dump",
  }
}

/// A signal sent to a process, or to the process group it leads.
pub fn signal_process(pid: u32, group: bool, signal: i32) {
  let Ok(pid) = i32::try_from(pid) else { return };

  if pid <= 0 {
    return;
  }

  // SAFETY: kill only sends a signal; a process or group that is gone answers with an error left unread.
  unsafe {
    libc::kill(if group { -pid } else { pid }, signal);
  }
}

/// The health a deploy tree's declaration names, read from the release as the deployment reads it.
pub fn declared_health(tree: &Path) -> Option<String> {
  let text = std::fs::read_to_string(tree.join("testbed.toml")).ok()?;

  Declaration::parse("testbed.toml", &text).ok()?.deploy.health
}

fn spread(span: Duration) -> Duration {
  if span.is_zero() {
    return span;
  }

  let seed = u64::from(chrono::Utc::now().timestamp_subsec_nanos()) ^ u64::from(std::process::id());

  Duration::from_millis(seed % (span.as_millis().max(1) as u64))
}

fn now_ms() -> i64 {
  chrono::Utc::now().timestamp_millis()
}

/// The state file as the supervisor last wrote it, where it reads.
pub fn read_snapshot(file: &Path) -> Option<Snapshot> {
  serde_json::from_str(&std::fs::read_to_string(file).ok()?).ok()
}

fn write_snapshot(file: &Path, snapshot: &Snapshot) {
  let Ok(text) = serde_json::to_string_pretty(snapshot) else { return };
  let written = file.with_extension("json.new");

  if std::fs::write(&written, text).is_ok() {
    let _ = std::fs::rename(&written, file);
  }
}
