//! The account this process runs as, named as a login names it.

/// The account's name: `USER` where the environment names one, else the password database's name for this
/// process's user id. A container's command or a process started with a bare environment has no `USER`, where a
/// login and a user manager always give one; empty where neither says.
pub fn account_name() -> String {
  match std::env::var("USER") {
    Ok(name) if !name.is_empty() => name,
    _ => password_name().unwrap_or_default(),
  }
}

fn password_name() -> Option<String> {
  let mut buffer = vec![0_u8; 4096];
  // SAFETY: an all-zero passwd is a valid value for getpwuid_r to fill.
  let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
  let mut found: *mut libc::passwd = std::ptr::null_mut();
  // SAFETY: getuid has no preconditions; getpwuid_r writes only into `entry` and `buffer`, whose length is
  // given, and sets `found` to `entry` or to null.
  let code = unsafe {
    libc::getpwuid_r(
      libc::getuid(),
      &raw mut entry,
      buffer.as_mut_ptr().cast(),
      buffer.len(),
      &raw mut found,
    )
  };

  if code != 0 || found.is_null() || entry.pw_name.is_null() {
    return None;
  }

  // SAFETY: on success pw_name points at a NUL-terminated string inside `buffer`, which outlives this read.
  let name = unsafe { std::ffi::CStr::from_ptr(entry.pw_name) };

  Some(name.to_string_lossy().into_owned())
}
