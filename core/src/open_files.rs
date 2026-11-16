//! The limit on open files every verb raises as it starts (docs/testbed.md, 「Open files」): the soft limit set
//! to the hard one, so whatever the verb starts inherits the machine's ceiling rather than a shell's.

/// Raises this process's soft limit on open files to its hard limit; a limit that cannot be read or raised is
/// left as it stands.
pub fn raise_open_files() {
  let mut limit = libc::rlimit {
    rlim_cur: 0,
    rlim_max: 0,
  };

  // SAFETY: reads this process's own limit into a value of the right shape.
  if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &raw mut limit) } != 0
    || limit.rlim_cur >= limit.rlim_max
  {
    return;
  }

  limit.rlim_cur = limit.rlim_max;
  // SAFETY: sets this process's own soft limit to the hard limit it already holds.
  unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &raw const limit) };
}
