//! The machine's deployment line (docs/deploy.md, 「The deployment line」): every deployment stands in it, the
//! first to ask going first, and holds it from its gate to the end of its build, so deployments asked together
//! pass their gate and their build one after the other. A deployment marked first stands ahead of every one
//! waiting unmarked, never ahead of the one holding the line, and a waiting one is raised to first by a mark its
//! unit names. A deployment waiting there says where it stands, and once it has waited the bound it goes on
//! beside the holder, so the wait never fails it. Before the line, a deployment waits for its deploy root's
//! turn, which keeps one deployment of a deploy root under way at a time.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::time::Duration;

use bmo_testbed_core::policy::deploy::DEPLOY;
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::lanes::LANES;
use bmo_testbed_core::queue::{HeldLock, LockWait, QueueTicket, Rank, now_ms, queue_tickets, take_lock};
use bmo_testbed_core::verb::age;
use serde::{Deserialize, Serialize};

/// The line's lock under the testbed data root's `locks/`.
pub const DEPLOY_LINE_LOCK: &str = "deploy-line.lock";
/// The folder beside the line's lock holding the first mark of each waiting deployment raised to first, a file
/// for each, named by its unit.
pub const FIRST_MARKS: &str = "deploy-line.first";

/// Who marked a deployment first, why, and when, in milliseconds since the epoch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirstMark {
  pub by: String,
  pub why: String,
  pub at: i64,
}

impl FirstMark {
  /// A mark made now.
  pub fn now(by: &str, why: &str) -> Self {
    Self {
      by: by.to_owned(),
      why: why.to_owned(),
      at: now_ms(),
    }
  }
}

/// A deployment as its ticket stands for it, in the line or for its deploy root's turn (docs/boundary.md,
/// 「`status`」): the words naming it, what they are made of, and its first mark where it has one. A ticket an
/// earlier revision stood holds its words alone.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployTicket {
  pub words: String,
  pub repository: Option<String>,
  pub commit: Option<String>,
  pub unit: Option<String>,
  pub asked_at: Option<i64>,
  /// The process standing the ticket, as the queue names it.
  #[serde(default)]
  pub pid: i64,
  pub first: Option<FirstMark>,
  /// It was asked with a command to run before its start, which runs only in its own restart.
  #[serde(default)]
  pub before_start: bool,
}

impl DeployTicket {
  /// The ticket of a deployment of a repository's short commit asked now, in its unit where it runs in one.
  pub fn asked(repository: &str, commit: &str, unit: Option<&str>) -> Self {
    Self {
      words: format!("{repository} {commit}{}", unit.map(|unit| format!(" ({unit})")).unwrap_or_default()),
      repository: Some(repository.to_owned()),
      commit: Some(commit.to_owned()),
      unit: unit.map(str::to_owned),
      asked_at: Some(now_ms()),
      pid: 0,
      first: None,
      before_start: false,
    }
  }

  /// The ticket a queue's words stand for: one this testbed stood, or the words alone of an earlier one.
  pub fn read(text: &str) -> Self {
    serde_json::from_str(text).unwrap_or_else(|_| Self {
      words: text.to_owned(),
      ..Self::default()
    })
  }

  // The ticket as a queue's words hold it.
  fn written(&self) -> String {
    serde_json::to_string(self).unwrap_or_else(|_| self.words.clone())
  }

  /// The words a line names it by, with its first mark where it has one.
  pub fn named(&self) -> String {
    match &self.first {
      Some(first) => fill_marks(
        &DEPLOY.run.first_named,
        &[("words", &self.words), ("by", &first.by), ("why", &first.why)],
      ),
      None => self.words.clone(),
    }
  }

  /// Whether it is a deployment of the repository and commit named, either commit the other's start.
  pub fn is_of(&self, repository: &str, commit: &str) -> bool {
    let asked = commit.to_ascii_lowercase();

    self.repository.as_deref() == Some(repository)
      && self.commit.as_deref().is_some_and(|own| {
        let own = own.to_ascii_lowercase();

        !own.is_empty() && !asked.is_empty() && (own.starts_with(&asked) || asked.starts_with(&own))
      })
  }

  fn of(ticket: &QueueTicket) -> Self {
    Self {
      pid: ticket.pid,
      ..Self::read(&ticket.holder)
    }
  }
}

/// A line's tickets as they stand: the one holding it, and those waiting in the order the line serves them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Standing {
  pub holder: Option<DeployTicket>,
  pub waiting: Vec<DeployTicket>,
}

/// The tickets under a lock, the line's or a deploy root's turn's, read now. The holder's is the one marked
/// seated, wherever it sorts; where none is, the first ticket an earlier revision stood, which marks none.
pub fn standing(lock: &Path) -> Standing {
  let tickets = queue_tickets(lock);
  let held = tickets.iter().position(|ticket| ticket.seated).or_else(|| {
    tickets
      .iter()
      .position(|ticket| DeployTicket::read(&ticket.holder).repository.is_none())
  });
  let holder = held.map(|at| DeployTicket::of(&tickets[at]));
  let waiting = tickets
    .iter()
    .enumerate()
    .filter(|(at, _)| Some(*at) != held)
    .map(|(_, ticket)| DeployTicket::of(ticket))
    .collect();

  Standing { holder, waiting }
}

/// The commits the deployments waiting under a lock ask for, of the repository named, in the order the lock
/// serves them: what a deployment whose turn came may put live in place of its own (docs/deploy.md, 「The
/// deployment line」). One asked with a command to run before its start names none.
pub fn waiting_commits(lock: &Path, repository: &str) -> Vec<String> {
  standing(lock)
    .waiting
    .into_iter()
    .filter(|ticket| ticket.repository.as_deref() == Some(repository) && !ticket.before_start)
    .filter_map(|ticket| ticket.commit)
    .collect()
}

/// The ticket of a deployment of the repository and commit waiting in a unit under any of the locks given, for
/// its deploy root's turn or in the line: the one a raise names by its unit.
pub fn waiting_ticket(locks: &[&Path], repository: &str, commit: &str) -> Option<DeployTicket> {
  locks
    .iter()
    .flat_map(|lock| standing(lock).waiting)
    .find(|ticket| ticket.is_of(repository, commit) && ticket.unit.is_some())
}

/// The line as `status` says it (docs/boundary.md, 「`status`」): its heading, the deployment holding it, and
/// each one waiting in the order the line serves them, with how long ago it asked and its first mark.
pub fn status_lines(lock: &Path, now: i64) -> Vec<String> {
  let said = &LANES.status;
  let line = standing(lock);
  let ticket_line =
    |ticket: &DeployTicket| format!("{}{}", ticket.named(), age(ticket.asked_at.unwrap_or(0), now));
  let holding = line.holder.as_ref().map_or_else(|| said.nobody.clone(), ticket_line);
  let mut lines = vec![said.deploy_line.clone(), fill_marks(&said.holding, &[("who", &holding)])];

  lines.extend(line.waiting.iter().enumerate().map(|(at, ticket)| {
    fill_marks(&said.waiting, &[("at", &(at + 1).to_string()), ("who", &ticket_line(ticket))])
  }));
  lines
}

/// What a raise of a waiting deployment asks, and how it learns where that deployment stands.
pub struct RaiseAsk<'a> {
  /// The line's lock.
  pub line: &'a Path,
  pub unit: &'a str,
  pub mark: FirstMark,
  /// Whether a unit is a running deployment unit.
  pub running: &'a dyn Fn(&str) -> bool,
  /// Whether the testbed's record holds a deployment run in the unit, one that has passed the line.
  pub recorded: &'a dyn Fn(&str) -> Result<bool, String>,
}

// The folder of first marks beside a line's lock.
fn marks_of(lock: &Path) -> PathBuf {
  lock.with_file_name(FIRST_MARKS)
}

/// A waiting deployment raised to first by its unit (docs/deploy.md, 「The deployment line」): the line saying
/// so, or why it is refused, with nothing written. Marks of units no longer running are taken away first.
pub fn raise_first(ask: &RaiseAsk<'_>) -> Result<String, String> {
  let said = &DEPLOY.run;
  let unit = ask.unit;
  let named = |line: &str| fill_marks(line, &[("unit", unit)]);
  // The unit names the mark's file, so it is a name and never a path.
  let plain = !unit.is_empty() && !unit.starts_with('.') && !unit.contains('/');

  if !plain {
    return Err(named(&said.raise_unit));
  }

  let marks = marks_of(ask.line);

  for gone in std::fs::read_dir(&marks).into_iter().flatten().flatten() {
    let file = gone.path();
    let stem = file.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();

    if !(ask.running)(&stem) {
      let _ = std::fs::remove_file(&file);
    }
  }

  let line = standing(ask.line);
  let mark = marks.join(format!("{unit}.json"));
  let waiting = line.waiting.iter().find(|ticket| ticket.unit.as_deref() == Some(unit));

  if line.holder.as_ref().and_then(|holder| holder.unit.as_deref()) == Some(unit) {
    return Err(named(&said.raise_holding));
  }

  if waiting.is_some_and(|ticket| ticket.first.is_some()) || mark.exists() {
    return Err(named(&said.raise_already));
  }

  // A deployment with no ticket in the line still waits for its deploy root's turn only while its unit runs
  // and the record holds nothing of it yet.
  if waiting.is_none() {
    if !(ask.running)(unit) {
      return Err(named(&said.raise_not_running));
    }

    match (ask.recorded)(unit) {
      Ok(false) => {}
      Ok(true) => return Err(named(&said.raise_past)),
      Err(why) => return Err(fill_marks(&said.raise_unread, &[("unit", unit), ("why", &why)])),
    }
  }

  let unwritten = |why: String| fill_marks(&said.raise_unwritten, &[("unit", unit), ("why", &why)]);
  let staged = marks.join(format!(".{unit}.json-{}", std::process::id()));
  let text = serde_json::to_string(&ask.mark).map_err(|why| unwritten(why.to_string()))?;

  // Staged beside and renamed in, so the deployment reading it never reads half a mark.
  std::fs::create_dir_all(&marks)
    .and_then(|()| std::fs::write(&staged, text))
    .and_then(|()| std::fs::rename(&staged, &mark))
    .map_err(|why| {
      let _ = std::fs::remove_file(&staged);

      unwritten(why.to_string())
    })?;

  Ok(fill_marks(
    &said.raise_done,
    &[("unit", unit), ("by", &ask.mark.by), ("why", &ask.mark.why)],
  ))
}

/// The deployment line a run stands in: its lock, how long a deployment waits before it goes on beside the
/// holder, how often it says where it stands while it waits, and how often it looks.
#[derive(Debug, Clone)]
pub struct DeployLine {
  pub lock: PathBuf,
  pub bound: Duration,
  pub say_every: Duration,
  /// How often a waiting deployment looks; none is the lanes' pace. A test sets it short.
  pub poll: Option<Duration>,
}

fn millis(duration: Duration) -> i64 {
  i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

// How many deployments the line serves before the one standing under this process's ticket: the holder, then
// those waiting ahead of it.
fn standing_ahead(lock: &Path) -> usize {
  let own = i64::from(std::process::id());
  let line = standing(lock);
  let holder = line.holder.is_some_and(|holder| holder.pid != own);

  usize::from(holder) + line.waiting.iter().take_while(|ticket| ticket.pid != own).count()
}

impl DeployLine {
  /// The machine's line under a testbed data root, at the policy's bound and pace.
  pub fn under(data_root: &Path) -> Self {
    let numbers = &DEPLOY.numbers;

    Self {
      lock: data_root.join("locks").join(DEPLOY_LINE_LOCK),
      bound: Duration::from_secs(numbers.line_bound_minutes.saturating_mul(60)),
      say_every: Duration::from_secs(numbers.line_say_seconds),
      poll: None,
    }
  }

  // The first mark a raise left for the ticket's unit, taken away as it is read; none where none was left.
  fn take_mark(&self, ticket: &DeployTicket) -> Option<FirstMark> {
    let mark = marks_of(&self.lock).join(format!("{}.json", ticket.unit.as_deref()?));
    let read = serde_json::from_str(&std::fs::read_to_string(&mark).ok()?).ok();

    let _ = std::fs::remove_file(&mark);
    read
  }

  /// The line's turn, stood for under the ticket given, each wait said on a line: held until it is dropped, or
  /// none where the bound passed first, said on a line naming the holder the deployment goes on beside. A
  /// ticket marked first stands ahead of every unmarked one, and one a raise marks while it waits moves there.
  pub fn take(&self, ticket: &DeployTicket, say: &dyn Fn(&str)) -> Option<HeldLock> {
    let said = &DEPLOY.run;
    let began = now_ms();
    let bound_at = began.saturating_add(millis(self.bound));
    let stood = RefCell::new(ticket.clone());

    if let Some(dir) = self.lock.parent() {
      let _ = std::fs::create_dir_all(dir);
    }

    // A raise made while the deployment waited for its deploy root's turn is read as it reaches the line, and
    // taken away all the same where the deployment was asked first already.
    let raised = self.take_mark(ticket);

    if let Some(first) = &ticket.first {
      say(&fill_marks(&said.first_marked, &[("by", &first.by), ("why", &first.why)]));
    } else if let Some(mark) = raised {
      say(&fill_marks(&said.first_raised, &[("by", &mark.by), ("why", &mark.why)]));
      stood.borrow_mut().first = Some(mark);
    }

    let rank = if stood.borrow().first.is_some() { Rank::Front } else { Rank::Ordinary };
    let mut wait_on = |ahead: Option<&str>| -> Option<i64> {
      let now = now_ms();
      let holding = standing(&self.lock)
        .holder
        .map(|holder| holder.named())
        .or_else(|| ahead.map(|ahead| DeployTicket::read(ahead).named()))
        .unwrap_or_else(|| said.line_nobody.clone());

      if now >= bound_at {
        let minutes = (self.bound.as_secs() / 60).to_string();

        say(&fill_marks(&said.line_bound, &[("minutes", &minutes), ("holder", &holding)]));

        return None;
      }

      let mut line = fill_marks(
        &said.line_waiting,
        &[
          ("seconds", &((now - began) / 1000).to_string()),
          ("ahead", &standing_ahead(&self.lock).to_string()),
          ("holder", &holding),
        ],
      );

      if let Some(first) = &stood.borrow().first {
        line.push_str(&fill_marks(&said.line_first, &[("by", &first.by), ("why", &first.why)]));
      }

      say(&line);

      Some(now.saturating_add(millis(self.say_every)).min(bound_at))
    };
    let mut raise = || -> Option<String> {
      let mark = self.take_mark(&stood.borrow())?;

      say(&fill_marks(&said.first_raised, &[("by", &mark.by), ("why", &mark.why)]));
      stood.borrow_mut().first = Some(mark);

      Some(stood.borrow().written())
    };
    let first = self.say_every.min(self.bound).as_secs_f64();
    // Read out before the wait, so no borrow of the ticket outlives this line while the raise marks it.
    let holder = stood.borrow().written();
    let held = take_lock(
      &self.lock,
      first,
      LockWait {
        holder,
        wait_on: Some(&mut wait_on),
        rank,
        poll: self.poll,
        raise: Some(&mut raise),
      },
    )?;

    held.mark_seated();

    let waited = (now_ms() - began) / 1000;

    if waited > 0 {
      say(&fill_marks(&said.line_taken, &[("seconds", &waited.to_string())]));
    }

    Some(held)
  }
}

/// The run lock under the deploy root, the deploy root's turn.
pub const DEPLOY_RUN_LOCK: &str = "deploy-run.lock";

/// A deploy root's turn (docs/deploy.md, 「The deployment line」): one deployment of a deploy root runs at a
/// time, the first to ask going first, each holding the turn to its end. A deployment waits for it with no
/// bound, saying where it stands at once and then at the pace given.
#[derive(Debug, Clone)]
pub struct RootTurn {
  pub lock: PathBuf,
  pub say_every: Duration,
  /// How often a waiting deployment looks; none is the lanes' pace. A test sets it short.
  pub poll: Option<Duration>,
}

impl RootTurn {
  /// The turn of a deploy root, said at the deployment line's pace.
  pub fn under(deploy_root: &Path) -> Self {
    Self {
      lock: deploy_root.join(DEPLOY_RUN_LOCK),
      say_every: Duration::from_secs(DEPLOY.numbers.line_say_seconds),
      poll: None,
    }
  }

  /// The deploy root's turn, stood for under the ticket given and held until it is dropped. The wait has no
  /// bound, so none comes back only where the pace is zero.
  pub fn take(&self, ticket: &DeployTicket, say: &dyn Fn(&str)) -> Option<HeldLock> {
    let said = &DEPLOY.run;
    let began = now_ms();

    if let Some(dir) = self.lock.parent() {
      let _ = std::fs::create_dir_all(dir);
    }

    let mut wait_on = |ahead: Option<&str>| -> Option<i64> {
      let now = now_ms();
      let holding =
        ahead.map_or_else(|| said.line_nobody.clone(), |ahead| DeployTicket::read(ahead).named());

      say(&fill_marks(
        &said.root_waiting,
        &[("seconds", &((now - began) / 1000).to_string()), ("holder", &holding)],
      ));

      Some(now.saturating_add(millis(self.say_every)))
    };
    let held = take_lock(
      &self.lock,
      0.0,
      LockWait {
        holder: ticket.written(),
        wait_on: Some(&mut wait_on),
        poll: self.poll,
        ..LockWait::default()
      },
    )?;

    held.mark_seated();

    let waited = (now_ms() - began) / 1000;

    if waited > 0 {
      say(&fill_marks(&said.root_taken, &[("seconds", &waited.to_string())]));
    }

    Some(held)
  }
}
