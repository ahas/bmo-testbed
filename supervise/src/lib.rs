//! The set's services run without systemd (docs/supervise.md): the unit files a deployment links read as systemd
//! reads them, their processes started, watched, restarted on a crash and stopped by one supervisor, which
//! answers a control socket, and stand-ins for `systemctl`, `systemd-run` and `journalctl` that ask it.

pub mod control;
pub mod env_file;
pub mod manager;
pub mod places;
pub mod serve;
pub mod stand_in;
pub mod state;
pub mod unit;
