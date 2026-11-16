//! The limit on open files every verb raises as it starts (docs/testbed.md, 「Open files」).

use bmo_testbed_core::open_files::raise_open_files;

fn open_files_limit() -> libc::rlimit {
  let mut limit = libc::rlimit {
    rlim_cur: 0,
    rlim_max: 0,
  };

  // SAFETY: reads this process's own limit into a value of the right shape.
  assert_eq!(unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &raw mut limit) }, 0);
  limit
}

// "the soft limit is set to the hard one": a process whose soft limit stands below its hard one is raised to it.
#[test]
fn the_soft_limit_on_open_files_is_raised_to_the_hard_one() {
  let mut lowered = open_files_limit();

  lowered.rlim_cur = lowered.rlim_max.min(256);
  // SAFETY: lowers this test process's own soft limit, which stays within its hard limit.
  assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &raw const lowered) }, 0);

  raise_open_files();

  let raised = open_files_limit();

  assert_eq!(raised.rlim_cur, raised.rlim_max);
}
