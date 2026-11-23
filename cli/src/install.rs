//! `install` (docs/install.md): the words read into what the run is asked, the run made against this machine, and
//! each step's line printed as it comes.

use std::path::PathBuf;

use bmo_testbed_core::turn::process_env;
use bmo_testbed_core::verb::data_root;
use bmo_testbed_install::{Ask, Host, Install, Line, Mode, Services, exit_code};
use bmo_testbed_suite::dependents::registry_path;

use crate::words::InstallArgs;

pub fn install(args: &InstallArgs) -> u8 {
  let mode = match (&args.preset, &args.add, &args.set) {
    (Some(name), _, _) => Mode::Preset {
      name: name.clone(),
      with: args.with.clone(),
    },
    (None, Some(repository), _) => Mode::Add(repository.clone()),
    (None, None, Some(input)) => Mode::Set(input.clone()),
    (None, None, None) => Mode::Status,
  };
  let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"));
  let prefix = args.prefix.clone().unwrap_or_else(|| home.join(".local/share"));
  let named = |given: &Option<String>, variable: &str, default: String| {
    given
      .clone()
      .or_else(|| std::env::var(variable).ok().filter(|value| !value.is_empty()))
      .unwrap_or(default)
  };
  let defaults = Services::default();
  let services = Services {
    postgres_host: named(&args.postgres_host, "PGHOST", defaults.postgres_host),
    postgres_port: named(&args.postgres_port, "PGPORT", defaults.postgres_port),
    valkey: named(&args.valkey, "BMO_INSTALL_VALKEY", defaults.valkey),
  };
  let data_root = args.data_root.clone().unwrap_or_else(data_root);
  let checkouts = registry_path(&data_root, &process_env());
  let ask = Ask {
    mode,
    index: args.index.clone(),
    data_root,
    home,
    user: bmo_testbed_core::account::account_name(),
    dry_run: args.dry_run,
    cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    prefix,
    services: services.clone(),
    checkouts,
  };
  let host = Host::new(&services.postgres_host, &services.postgres_port);
  let mut say = |line: &Line| println!("{}", line.json());
  let outcome = Install::new(ask, &host, &mut say).run();

  if let bmo_testbed_install::run::Outcome::Refused(why) = &outcome {
    eprintln!("{why}");
  }

  exit_code(&outcome)
}
