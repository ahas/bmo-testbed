//! The machine's deployment line (docs/deploy.md, 「The deployment line」): every deployment stands in it, the
//! first to ask going first, and holds it from its gate to the end of its build, so deployments asked together
//! pass their gate and their build one after the other. A deployment marked first stands ahead of every one
//! waiting unmarked, never ahead of the one holding the line, and a waiting one is raised to first by a mark its
//! unit names. A deployment waiting there says where it stands, and once it has waited the bound it goes on
//! beside the holder, so the wait never fails it. Before the line, a deployment waits for its deploy root's
//! turn, which keeps one deployment of a deploy root under way at a time.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::Duration;

use bmo_testbed_core::policy::deploy::DEPLOY;
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::lanes::LANES;
use bmo_testbed_core::queue::{HeldLock, LockWait, QueueTicket, Rank, now_ms, queue_tickets, take_lock};
use bmo_testbed_core::verb::age;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::tree::short;

/// The line's lock under the testbed data root's `locks/`.
pub const DEPLOY_LINE_LOCK: &str = "deploy-line.lock";
/// The folder beside the line's lock naming the lock of each deploy root's turn a deployment has stood for, a
/// file for each, so the line's whole service order is read without knowing any repository.
pub const ROOT_TURNS: &str = "deploy-line.roots";
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
  /// The whole commit it asks for; the short one where an earlier revision stood the ticket.
  pub commit: Option<String>,
  /// The deploy root it deploys.
  #[serde(default)]
  pub root: Option<String>,
  pub unit: Option<String>,
  pub asked_at: Option<i64>,
  /// The process standing the ticket, as the queue names it.
  #[serde(default)]
  pub pid: i64,
  pub first: Option<FirstMark>,
  /// It was asked with a command to run before its start, which runs only in its own restart.
  #[serde(default)]
  pub before_start: bool,
  /// It was asked with its gate left out, which stands on a pass of its own commit alone.
  #[serde(default)]
  pub no_check: bool,
}

impl DeployTicket {
  /// The ticket of a deployment of a repository's whole commit asked now, in its unit where it runs in one,
  /// its words naming the commit short.
  pub fn asked(repository: &str, commit: &str, unit: Option<&str>) -> Self {
    Self {
      words: format!(
        "{repository} {}{}",
        short(commit),
        unit.map(|unit| format!(" ({unit})")).unwrap_or_default()
      ),
      repository: Some(repository.to_owned()),
      commit: Some(commit.to_owned()),
      root: None,
      unit: unit.map(str::to_owned),
      asked_at: Some(now_ms()),
      pid: 0,
      first: None,
      before_start: false,
      no_check: false,
    }
  }

  /// The ticket a queue's words stand for: one this testbed stood, or the words alone of an earlier one.
  pub fn read(text: &str) -> Self {
    serde_json::from_str(text).unwrap_or_else(|_| Self {
      words: text.to_owned(),
      ..Self::default()
    })
  }

  /// The ticket as a queue's words hold it.
  pub fn written(&self) -> String {
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

// The folder beside a deploy root's turn the later marks stand in, one file a waiting unit.
const LATER_MARKS: &str = "deploy-later";

fn later_mark(turn: &Path, unit: &str) -> Option<PathBuf> {
  let plain = !unit.is_empty() && !unit.starts_with('.') && !unit.contains('/');

  plain.then(|| turn.with_file_name(LATER_MARKS).join(format!("{unit}.commit")))
}

/// A later commit left for a deployment of the deploy root still waiting, by its unit (docs/deploy.md, 「The
/// deployment line」): the deployment lays it down in place of its own once its turn comes. Whether it was left.
pub fn leave_later(turn: &Path, unit: &str, commit: &str) -> bool {
  let Some(mark) = later_mark(turn, unit) else { return false };
  let staged = mark.with_extension("staged");

  mark.parent().is_some_and(|dir| std::fs::create_dir_all(dir).is_ok())
    && std::fs::write(&staged, commit.as_bytes()).is_ok()
    && std::fs::rename(&staged, &mark).is_ok()
}

/// The later commit left for a unit, none where none was left.
pub fn later_left(turn: &Path, unit: &str) -> Option<String> {
  let text = std::fs::read_to_string(later_mark(turn, unit)?).ok()?;
  let commit = text.trim();

  (!commit.is_empty()).then(|| commit.to_owned())
}

/// The later commit left for a unit taken away, once its deployment has ended.
pub fn later_taken(turn: &Path, unit: &str) {
  if let Some(mark) = later_mark(turn, unit) {
    let _ = std::fs::remove_file(mark);
  }
}

/// The ticket of a deployment of the repository and commit waiting in a unit under any of the locks given, for
/// its deploy root's turn or in the line: the one a raise names by its unit.
pub fn waiting_ticket(locks: &[&Path], repository: &str, commit: &str) -> Option<DeployTicket> {
  locks
    .iter()
    .flat_map(|lock| standing(lock).waiting)
    .find(|ticket| ticket.is_of(repository, commit) && ticket.unit.is_some())
}

/// Where a waiting deployment waits: in the line, or before it for its deploy root's turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum WaitsFor {
  Line,
  Root,
}

/// A waiting deployment as the line will serve it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Waiting {
  #[serde(flatten)]
  pub ticket: DeployTicket,
  pub waits_for: WaitsFor,
}

/// The deployments asked of a line in the order it will serve them (docs/deploy.md, 「The deployment line」): the
/// one holding it, then those waiting in it, then those waiting for their deploy root's turn.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Served {
  pub holder: Option<DeployTicket>,
  pub waiting: Vec<Waiting>,
}

// Whether two tickets stand for one deployment: by their units where both name one, else by their processes.
fn same_deployment(one: &DeployTicket, other: &DeployTicket) -> bool {
  match (&one.unit, &other.unit) {
    (Some(one), Some(other)) => one == other,
    _ => one.pid == other.pid,
  }
}

/// The locks of the deploy roots' turns enrolled beside a line's lock, each once.
pub fn enrolled_turns(lock: &Path) -> Vec<PathBuf> {
  let mut turns: Vec<PathBuf> = std::fs::read_dir(lock.with_file_name(ROOT_TURNS))
    .into_iter()
    .flatten()
    .flatten()
    .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
    .map(|text| PathBuf::from(text.trim()))
    .filter(|turn| !turn.as_os_str().is_empty())
    .collect();

  turns.sort();
  turns.dedup();
  turns
}

// The first mark a raise left for the ticket's unit and the deployment has not read yet, left where it stands.
fn mark_left(lock: &Path, ticket: &DeployTicket) -> Option<FirstMark> {
  let mark = marks_of(lock).join(format!("{}.json", ticket.unit.as_deref()?));

  serde_json::from_str(&std::fs::read_to_string(mark).ok()?).ok()
}

// A deploy root's turn as the line's service order reads it: the deployment holding it, and those waiting for it.
struct RootQueue {
  holder: Option<DeployTicket>,
  waiting: VecDeque<DeployTicket>,
}

impl RootQueue {
  // The deploy root's next waiting deployment reaching the line as the one holding its turn ends: behind every one
  // waiting there, or, marked first, ahead of every unmarked one, after those marked before it. It then holds the
  // deploy root's turn.
  fn reach(&mut self, line: &mut VecDeque<Waiting>) {
    self.holder = self.waiting.pop_front();

    if let Some(ticket) = self.holder.clone() {
      let at = if ticket.first.is_some() {
        line.iter().take_while(|waiting| waiting.ticket.first.is_some()).count()
      } else {
        line.len()
      };

      line.insert(
        at,
        Waiting {
          ticket,
          waits_for: WaitsFor::Root,
        },
      );
    }
  }
}

/// The deployments asked of the line under a lock, read now in the order it will serve them (docs/deploy.md,
/// 「The deployment line」), the line followed one turn at a time. A holder lets the line go after its build and its
/// deploy root's turn only at its end, so the line passes to the next one waiting there before that deploy root's
/// next waiting deployment reaches it; a deploy root whose deployment stands nowhere in the line is past it, so its
/// next one reaches the line at once.
pub fn served(lock: &Path) -> Served {
  let line = standing(lock);
  let in_line =
    |ticket: &DeployTicket| line.holder.iter().chain(&line.waiting).any(|one| same_deployment(one, ticket));
  let mut roots: Vec<RootQueue> = enrolled_turns(lock)
    .iter()
    .map(|turn| {
      let root = standing(turn);
      let waiting = root
        .waiting
        .into_iter()
        .filter(|ticket| !in_line(ticket))
        .map(|ticket| DeployTicket {
          first: ticket.first.clone().or_else(|| mark_left(lock, &ticket)),
          ..ticket
        })
        .collect();

      RootQueue {
        holder: root.holder,
        waiting,
      }
    })
    .filter(|root| !root.waiting.is_empty())
    .collect();
  let mut queue: VecDeque<Waiting> = line
    .waiting
    .iter()
    .cloned()
    .map(|ticket| Waiting {
      ticket,
      waits_for: WaitsFor::Line,
    })
    .collect();

  for root in &mut roots {
    if !root.holder.as_ref().is_some_and(in_line) {
      root.reach(&mut queue);
    }
  }

  let mut serving = line.holder.clone();
  let mut waiting = Vec::new();

  loop {
    let next = queue.pop_front();

    if let Some(ended) = serving.take() {
      for root in &mut roots {
        if root.holder.as_ref().is_some_and(|holder| same_deployment(holder, &ended)) {
          root.reach(&mut queue);
        }
      }
    }

    let Some(next) = next.or_else(|| queue.pop_front()) else {
      break;
    };

    serving = Some(next.ticket.clone());
    waiting.push(next);
  }

  Served {
    holder: line.holder,
    waiting,
  }
}

/// The line as `status` says it (docs/boundary.md, 「`status`」): its heading, the deployment holding it, and
/// each one waiting in the order the line will serve them, with how long ago it asked, its first mark, and
/// whether it waits for its deploy root's turn.
pub fn status_lines(lock: &Path, now: i64) -> Vec<String> {
  let said = &LANES.status;
  let line = served(lock);
  let ticket_line =
    |ticket: &DeployTicket| format!("{}{}", ticket.named(), age(ticket.asked_at.unwrap_or(0), now));
  let holding = line.holder.as_ref().map_or_else(|| said.nobody.clone(), ticket_line);
  let mut lines = vec![said.deploy_line.clone(), fill_marks(&said.holding, &[("who", &holding)])];

  lines.extend(line.waiting.iter().enumerate().map(|(at, waiting)| {
    let who = match waiting.waits_for {
      WaitsFor::Line => ticket_line(&waiting.ticket),
      WaitsFor::Root => format!("{}{}", ticket_line(&waiting.ticket), said.for_root),
    };

    fill_marks(&said.waiting, &[("at", &(at + 1).to_string()), ("who", &who)])
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
  /// A deploy root's turn enrolled beside the line, by a file named for its lock's path and naming it, so the
  /// line's service order reads the deployments waiting for it (docs/deploy.md, 「The deployment line」). One
  /// that cannot be written leaves that deploy root's waiting deployments out of the order and nothing else.
  pub fn enrol(&self, turn: &Path) {
    let dir = self.lock.with_file_name(ROOT_TURNS);
    let path = turn.to_string_lossy();
    let name: String = Sha256::digest(path.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect();
    let enrolled = dir.join(&name);

    if enrolled.exists() {
      return;
    }

    let staged = dir.join(format!(".{name}-{}", std::process::id()));
    let written = std::fs::create_dir_all(&dir)
      .and_then(|()| std::fs::write(&staged, path.as_bytes()))
      .and_then(|()| std::fs::rename(&staged, &enrolled));

    if written.is_err() {
      let _ = std::fs::remove_file(&staged);
    }
  }

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
