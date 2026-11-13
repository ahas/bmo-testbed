//! The machine's deployment line (docs/deploy.md, 「The deployment line」): every deployment stands in it, the
//! first to ask going first, and holds it from its gate to the end of its build, so deployments asked together
//! pass their gate and their build one after the other. A deployment waiting there says where it stands, and
//! once it has waited the bound it goes on beside the holder, so the wait never fails it.

use std::path::{Path, PathBuf};
use std::time::Duration;

use bmo_testbed_core::policy::deploy::DEPLOY;
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::queue::{HeldLock, LockWait, now_ms, queue_tickets, take_lock};

/// The line's lock under the testbed data root's `locks/`.
pub const DEPLOY_LINE_LOCK: &str = "deploy-line.lock";

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

// How many living tickets stand ahead of this process's own in the line: the holder's among them.
fn standing_ahead(lock: &Path) -> usize {
  let own = i64::from(std::process::id());

  queue_tickets(lock).iter().take_while(|ticket| ticket.pid != own).count()
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

  /// The line's turn, stood for under the holder's name, each wait said on a line: held until it is dropped,
  /// or none where the bound passed first, said on a line naming the holder the deployment goes on beside.
  pub fn take(&self, holder: &str, say: &dyn Fn(&str)) -> Option<HeldLock> {
    let said = &DEPLOY.run;
    let began = now_ms();
    let bound_at = began.saturating_add(millis(self.bound));

    if let Some(dir) = self.lock.parent() {
      let _ = std::fs::create_dir_all(dir);
    }

    let mut wait_on = |ahead: Option<&str>| -> Option<i64> {
      let now = now_ms();
      let holding = ahead.map_or_else(|| said.line_nobody.clone(), str::to_owned);

      if now >= bound_at {
        let minutes = (self.bound.as_secs() / 60).to_string();

        say(&fill_marks(&said.line_bound, &[("minutes", &minutes), ("holder", &holding)]));

        return None;
      }

      say(&fill_marks(
        &said.line_waiting,
        &[
          ("seconds", &((now - began) / 1000).to_string()),
          ("ahead", &standing_ahead(&self.lock).to_string()),
          ("holder", &holding),
        ],
      ));

      Some(now.saturating_add(millis(self.say_every)).min(bound_at))
    };
    let first = self.say_every.min(self.bound).as_secs_f64();
    let held = take_lock(
      &self.lock,
      first,
      LockWait {
        holder: holder.to_owned(),
        wait_on: Some(&mut wait_on),
        poll: self.poll,
        ..LockWait::default()
      },
    )?;
    let waited = (now_ms() - began) / 1000;

    if waited > 0 {
      say(&fill_marks(&said.line_taken, &[("seconds", &waited.to_string())]));
    }

    Some(held)
  }
}
