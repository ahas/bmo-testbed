//! A sweep over a run's units, a sweep's width at a time (docs/suite.md, 「One sweep, a process per
//! file」). The units that must run alone go down a lane of one per kind, each taking one place of the width
//! and starting with the sweep, so they neither wait for the rest nor stand beside one another; once a lane
//! is empty it takes from the rest like any other place. The rest always get at least one place of their
//! own. Every unit is started and run to its end.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;

/// How a sweep is to go over its items.
pub struct SweepPlan<'a, Item> {
  /// How many items run at once.
  pub width: usize,
  /// The lane of one an item runs in, never beside another item of that lane; none for the rest.
  pub lane: &'a dyn Fn(&Item) -> Option<String>,
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
  let take = |queue: &Mutex<VecDeque<usize>>| loop {
    let Some(at) = queue.lock().expect("the queue is held only to take from it").pop_front() else {
      return;
    };

    let result = run(&items[at]);

    results.lock().expect("the results are held only to set one")[at] = Some(result);
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
