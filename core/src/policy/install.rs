//! What the install verb says and paces by (docs/install.md): its usage, each line's detail and next step, and
//! its refusals, with the `{name}` marks each names.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Install {
  pub usage: String,
  pub upgrade_usage: String,
  pub numbers: Numbers,
  pub next: Next,
  pub detail: Detail,
  pub upgrade: Upgrade,
  pub refusal: Refusal,
}

/// What the `upgrade` verb's lines say (docs/install.md, 「Upgrading: `upgrade`」).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Upgrade {
  pub target_release: String,
  pub target_nightly: String,
  pub release: String,
  pub nightly: String,
  pub checkout_stands: String,
  pub checkout_moved: String,
  pub checkout_planned: String,
  pub checkout_absent: String,
  pub checkout_dirty: String,
  pub checkout_branch: String,
  pub checkout_diverged: String,
  pub checkout_no_release: String,
  pub checkout_failed: String,
  pub checkouts_stopped: String,
  pub deploy_live: String,
  pub deploy_done: String,
  pub deploy_planned: String,
  pub deploy_failed: String,
  pub message: String,
  pub message_release: String,
  pub message_nightly: String,
  pub done: String,
  pub done_planned: String,
  pub done_failed: String,
  pub rerun: String,
  pub rerun_nightly: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Numbers {
  pub deployment_poll_seconds: u64,
  pub token_bytes: usize,
  pub quoted_chars: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Next {
  pub rerun: String,
  pub set: String,
  pub choose: String,
  pub read_declaration: String,
  pub install_program: String,
  pub mend_and_rerun: String,
  pub journal: String,
  pub open: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Detail {
  pub preset: String,
  pub choice_open: String,
  pub clone_made: String,
  pub clone_stands: String,
  pub clone_planned: String,
  pub clone_failed: String,
  pub checkouts_stand: String,
  pub checkouts_planned: String,
  pub checkouts_made: String,
  pub checkouts_failed: String,
  pub declaration: String,
  pub declaration_missing: String,
  pub declaration_unread: String,
  pub testbed_unread: String,
  pub name_twice: String,
  pub need_stands: String,
  pub need_missing: String,
  pub need_built: String,
  pub need_build_planned: String,
  pub need_build_failed: String,
  pub port_free: String,
  pub port_kept: String,
  pub port_moved: String,
  pub place_outside: String,
  pub port_taken: String,
  pub input_given: String,
  pub input_default: String,
  pub input_optional: String,
  pub input_needed: String,
  pub token_made: String,
  pub token_stands: String,
  pub token_planned: String,
  pub token_failed: String,
  pub database_made: String,
  pub database_stands: String,
  pub database_planned: String,
  pub database_failed: String,
  pub settings_written: String,
  pub settings_stands: String,
  pub settings_planned: String,
  pub settings_waiting: String,
  pub settings_unknown: String,
  pub settings_failed: String,
  pub line_added: String,
  pub line_stands: String,
  pub line_planned: String,
  pub line_waiting: String,
  pub features_written: String,
  pub features_stands: String,
  pub features_planned: String,
  pub deploy_none: String,
  pub deploy_live: String,
  pub deploy_done: String,
  pub deploy_planned: String,
  pub deploy_failed: String,
  pub restart_done: String,
  pub restart_failed: String,
  pub restart_none: String,
  pub after_done: String,
  pub after_ran: String,
  pub after_planned: String,
  pub after_failed: String,
  pub health_ok: String,
  pub health_none: String,
  pub health_failed: String,
  pub verify_ok: String,
  pub verify_failed: String,
  pub verify_planned: String,
  pub status: String,
  pub status_cloned: String,
  pub status_absent: String,
  pub status_deployed: String,
  pub status_undeployed: String,
  pub done: String,
  pub done_planned: String,
  pub done_waiting: String,
  pub done_failed: String,
  pub set: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Refusal {
  pub no_index: String,
  pub presets_unread: String,
  pub no_preset: String,
  pub no_repository: String,
  pub extends_ring: String,
  pub choice_many: String,
  pub no_install: String,
  pub no_input: String,
  pub set_failed: String,
  pub words: String,
}

super::part!(pub INSTALL: Install = "install.toml");
