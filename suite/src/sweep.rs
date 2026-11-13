//! A sweep over a run's units, a sweep's width at a time (docs/suite.md, 「One sweep, a process per
//! file」). The units that must run alone go down a lane of one per kind, each taking one place of the width
//! and starting with the sweep, so they neither wait for the rest nor stand beside one another; once a lane
//! is empty it takes from the rest like any other place. The rest always get at least one place of their
//! own. An item may name another it starts after: its place passes it over for the next item that waits on
//! nothing, and waits only where every item left to it does. Every unit is started and run to its end.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Condvar, Mutex};

/// How a sweep is to go over its items.
pub struct SweepPlan<'a, Item> {
  /// How many items run at once.
  pub width: usize,
  /// The lane of one an item runs in, never beside another item of that lane; none for the rest.
  pub lane: &'a dyn Fn(&Item) -> Option<String>,
  /// The position among the items of the one an item starts after, once that one has ended; none for an item
  /// that waits on nothing. An item named in its own queue behind the one waiting on it is taken first.
  pub after: &'a dyn Fn(&Item) -> Option<usize>,
}

/// The order a sweep starts files in: the largest first, files of the same size in the order they came.
pub fn largest_first(files: Vec<String>, size_of: impl Fn(&str) -> u64) -> Vec<String> {
  let mut sized: Vec<(u64, String)> = files.into_iter().map(|file| (size_of(&file), file)).collect();

  sized.sort_by_key(|(size, _)| std::cmp::Reverse(*size));
  sized.into_iter().map(|(_, file)| file).collect()
}

/// Runs the work over every item, each to its end, and gives the results back in the items' own order.
pub fn sweep<Item: Sync, Result: Send>(
  items: &[Item],
  plan: &SweepPlan<'_, Item>,
  run: &(dyn Fn(&Item) -> Result + Sync),
) -> Vec<Result> {
  let mut lanes: BTreeMap<String, VecDeque<usize>> = BTreeMap::new();
  let mut shared: VecDeque<usize> = VecDeque::new();

  for (at, item) in items.iter().enumerate() {
    match (plan.lane)(item) {
      Some(lane) => lanes.entry(lane).or_default().push_back(at),
      None => shared.push_back(at),
    }
  }

  let shared_width = shared.len().min(plan.width.saturating_sub(lanes.len()).max(1));
  let shared = Mutex::new(shared);
  let results: Mutex<Vec<Option<Result>>> = Mutex::new((0..items.len()).map(|_| None).collect());
  // Which items have ended, and the signal that one more has.
  let ended: Mutex<Vec<bool>> = Mutex::new(vec![false; items.len()]);
  let one_ended = Condvar::new();
  // An item is ready once the one it starts after has ended; one naming itself, or no item, waits on nothing.
  let befores: Vec<Option<usize>> = items.iter().map(plan.after).collect();
  let ready = |at: usize, ended: &[bool]| {
    befores[at].is_none_or(|before| before == at || ended.get(before).is_none_or(|done| *done))
  };
  // The next ready item of a queue, waiting for another to end while the queue holds only items that are not.
  let next = |queue: &Mutex<VecDeque<usize>>| -> Option<usize> {
    let mut so_far = ended.lock().expect("the ended items are held only to read or set one");

    loop {
      let mut waiting = queue.lock().expect("the queue is held only to take from it");

      if waiting.is_empty() {
        return None;
      }

      if let Some(place) = waiting.iter().position(|at| ready(*at, &so_far)) {
        return waiting.remove(place);
      }

      drop(waiting);
      so_far = one_ended.wait(so_far).expect("the ended items are held only to read or set one");
    }
  };
  let take = |queue: &Mutex<VecDeque<usize>>| {
    while let Some(at) = next(queue) {
      let result = run(&items[at]);

      results.lock().expect("the results are held only to set one")[at] = Some(result);
      ended.lock().expect("the ended items are held only to read or set one")[at] = true;
      one_ended.notify_all();
    }
  };
  let lanes: Vec<Mutex<VecDeque<usize>>> = lanes.into_values().map(Mutex::new).collect();

  std::thread::scope(|scope| {
    for lane in &lanes {
      scope.spawn(|| {
        take(lane);
        take(&shared);
      });
    }

    for _ in 0..shared_width {
      scope.spawn(|| take(&shared));
    }
  });

  results.into_inner().unwrap_or_default().into_iter().flatten().collect()
}
