//! What the install asks of the machine it runs on: programs, commands, git, PostgreSQL, ports, randomness and
//! a deployment waited out. The run reads and writes its files itself; everything else goes through this, so a
//! test stands a machine of its own in its place.

use std::io::Read;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use bmo_testbed_core::policy::install::INSTALL;

/// What a command came to: its exit code, -1 where it could not start or ended by a signal, and what it said on
/// its output and its error stream together.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ran {
  pub code: i32,
  pub said: String,
}

impl Ran {
  pub fn ok(&self) -> bool {
    self.code == 0
  }
}

pub trait Machine {
  /// Whether a program stands on the `PATH`.
  fn has_program(&self, program: &str) -> bool;
  /// A command run through `sh -c` from a directory, with variables added to the environment, waited for.
  fn shell(&self, command: &str, dir: &Path, env: &[(String, String)]) -> Ran;
  fn clone_repository(&self, url: &str, into: &Path) -> Ran;
  fn fetch(&self, checkout: &Path) -> Ran;
  /// The checkout moved onto a tag or a commit it holds.
  fn check_out(&self, checkout: &Path, rev: &str) -> Ran;
  /// The commit the remote's main branch stands at, as the checkout last fetched it.
  fn main_head(&self, checkout: &Path, main: &str) -> Option<String>;
  /// The commit a name of the checkout's stands for: a tag, a branch or `HEAD`.
  fn commit_of(&self, checkout: &Path, rev: &str) -> Option<String>;
  /// Whether the checkout's working tree holds changes not committed, tracked or not.
  fn has_changes(&self, checkout: &Path) -> bool;
  /// The branch the checkout stands on; none where it stands on a commit alone.
  fn branch(&self, checkout: &Path) -> Option<String>;
  /// Every tag the checkout holds.
  fn tags(&self, checkout: &Path) -> Vec<String>;
  /// The checkout's main branch moved forward to its remote's head, never merged.
  fn fast_forward(&self, checkout: &Path, main: &str) -> Ran;
  fn database_exists(&self, name: &str) -> Result<bool, String>;
  fn create_database(&self, name: &str) -> Ran;
  /// Whether nothing listens on the port of the loopback address.
  fn port_free(&self, port: u16) -> bool;
  fn random(&self, bytes: usize) -> Result<Vec<u8>, String>;
  /// A repository's commit put live from its checkout, waited for until its deployment unit ends; the unit's name
  /// where one was started. A first release goes live at once, since nothing of it is live to wait quiet for; a
  /// later one waits for quiet (docs/install.md, 「Deploying」 and 「Upgrading: `upgrade`」).
  fn deploy(&self, checkout: &Path, commit: &str, message: &str, first: bool) -> (Ran, Option<String>);
}

/// This machine.
pub struct Host {
  /// The testbed binary a deployment is asked of: the one running.
  pub testbed: PathBuf,
  /// PostgreSQL's host and port, which the database steps ask.
  pub postgres: (String, String),
}

impl Host {
  pub fn new(postgres_host: &str, postgres_port: &str) -> Self {
    let testbed = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("bmo-testbed"));

    Self {
      testbed,
      postgres: (postgres_host.to_owned(), postgres_port.to_owned()),
    }
  }
}

fn ran(command: &mut Command) -> Ran {
  match command.stdin(Stdio::null()).output() {
    Ok(output) => {
      let mut said = String::from_utf8_lossy(&output.stdout).into_owned();

      said.push_str(&String::from_utf8_lossy(&output.stderr));
      Ran {
        code: output.status.code().unwrap_or(-1),
        said: said.trim().to_owned(),
      }
    }
    Err(why) => Ran {
      code: -1,
      said: why.to_string(),
    },
  }
}

impl Machine for Host {
  fn has_program(&self, program: &str) -> bool {
    ran(Command::new("sh").arg("-c").arg("command -v \"$1\"").arg("sh").arg(program)).ok()
  }

  fn shell(&self, command: &str, dir: &Path, env: &[(String, String)]) -> Ran {
    ran(
      Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(dir)
        .envs(env.iter().map(|(name, value)| (name, value))),
    )
  }

  fn clone_repository(&self, url: &str, into: &Path) -> Ran {
    if let Some(parent) = into.parent() {
      let _ = std::fs::create_dir_all(parent);
    }

    ran(Command::new("git").arg("clone").arg("--quiet").arg(url).arg(into))
  }

  fn fetch(&self, checkout: &Path) -> Ran {
    ran(Command::new("git").arg("-C").arg(checkout).arg("fetch").arg("--quiet").arg("origin"))
  }

  fn check_out(&self, checkout: &Path, rev: &str) -> Ran {
    ran(
      Command::new("git")
        .arg("-C")
        .arg(checkout)
        .args(["-c", "advice.detachedHead=false", "checkout", "--quiet"])
        .arg(rev),
    )
  }

  fn main_head(&self, checkout: &Path, main: &str) -> Option<String> {
    let head = ran(
      Command::new("git")
        .arg("-C")
        .arg(checkout)
        .arg("rev-parse")
        .arg(format!("origin/{main}^{{commit}}")),
    );

    head.ok().then_some(head.said)
  }

  fn commit_of(&self, checkout: &Path, rev: &str) -> Option<String> {
    let commit = ran(
      Command::new("git")
        .arg("-C")
        .arg(checkout)
        .args(["rev-parse", "--verify", "--quiet"])
        .arg(format!("{rev}^{{commit}}")),
    );

    commit.ok().then(|| commit.said.trim().to_owned())
  }

  fn has_changes(&self, checkout: &Path) -> bool {
    let status = ran(Command::new("git").arg("-C").arg(checkout).args(["status", "--porcelain"]));

    !status.ok() || !status.said.trim().is_empty()
  }

  fn branch(&self, checkout: &Path) -> Option<String> {
    let branch =
      ran(
        Command::new("git")
          .arg("-C")
          .arg(checkout)
          .args(["symbolic-ref", "--quiet", "--short", "HEAD"]),
      );

    branch.ok().then(|| branch.said.trim().to_owned()).filter(|name| !name.is_empty())
  }

  fn tags(&self, checkout: &Path) -> Vec<String> {
    let tags = ran(Command::new("git").arg("-C").arg(checkout).args(["tag", "--list"]));

    tags
      .said
      .lines()
      .map(str::trim)
      .filter(|tag| !tag.is_empty())
      .map(str::to_owned)
      .collect()
  }

  fn fast_forward(&self, checkout: &Path, main: &str) -> Ran {
    ran(
      Command::new("git")
        .arg("-C")
        .arg(checkout)
        .args(["merge", "--ff-only", "--quiet"])
        .arg(format!("origin/{main}")),
    )
  }

  fn database_exists(&self, name: &str) -> Result<bool, String> {
    // The name is handed to psql as a variable its script quotes, read from standard input, since psql fills a
    // variable only in a script it reads.
    let mut child = Command::new("psql")
      .env("PGHOST", &self.postgres.0)
      .env("PGPORT", &self.postgres.1)
      .args(["--dbname=postgres", "--no-psqlrc", "--tuples-only", "--no-align", "--quiet"])
      .arg(format!("--variable=name={name}"))
      .stdin(Stdio::piped())
      .stdout(Stdio::piped())
      .stderr(Stdio::piped())
      .spawn()
      .map_err(|why| why.to_string())?;

    if let Some(stdin) = child.stdin.as_mut() {
      use std::io::Write;
      let _ = stdin.write_all(b"select 1 from pg_database where datname = :'name';\n");
    }

    let output = child.wait_with_output().map_err(|why| why.to_string())?;

    if !output.status.success() {
      return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }

    Ok(String::from_utf8_lossy(&output.stdout).lines().any(|line| line.trim() == "1"))
  }

  fn create_database(&self, name: &str) -> Ran {
    ran(
      Command::new("createdb")
        .env("PGHOST", &self.postgres.0)
        .env("PGPORT", &self.postgres.1)
        .arg(name),
    )
  }

  fn port_free(&self, port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_ok()
  }

  fn random(&self, bytes: usize) -> Result<Vec<u8>, String> {
    let mut out = vec![0; bytes];

    std::fs::File::open("/dev/urandom")
      .and_then(|mut source| source.read_exact(&mut out))
      .map_err(|why| why.to_string())?;
    Ok(out)
  }

  fn deploy(&self, checkout: &Path, commit: &str, message: &str, first: bool) -> (Ran, Option<String>) {
    // An install's or an upgrade's deployment is not a harness's deployment (docs/install.md, 「Deploying」 and
    // 「Upgrading: `upgrade`」): the harness's mark is not handed on, so the deploy verb takes it.
    // The commands a deployment's steps call, `bmo` and each unit's own, are linked into `.local/bin` under the home
    // directory, which a fresh machine's shell may not hold on its PATH yet.
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut paths = vec![home.join(".local/bin")];

    paths.extend(std::env::split_paths(&path));

    let mut words = vec!["deploy", "--no-check"];

    if first {
      words.push("--force-live");
    }

    words.extend(["--commit", commit, "--message", message]);

    let started = ran(
      Command::new(&self.testbed)
        .args(&words)
        .current_dir(checkout)
        .env("PATH", std::env::join_paths(paths).unwrap_or(path))
        .env_remove("CLAUDE_PID"),
    );
    let unit = started
      .said
      .split_whitespace()
      .map(|word| word.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-'))
      .find(|word| word.starts_with("bmo-deploy-"))
      .map(str::to_owned);

    if !started.ok() {
      return (started, unit);
    }

    let Some(unit) = unit else {
      return (started, None);
    };

    // The unit is waited for until it ends, asked again at the policy's pace, however long it runs.
    loop {
      let state =
        ran(Command::new("systemctl").args(["--user", "show", "--property=ActiveState", "--value", &unit]));
      let state = state.said.trim().to_owned();

      if state != "active" && state != "activating" && state != "deactivating" && state != "reloading" {
        break;
      }

      std::thread::sleep(Duration::from_secs(INSTALL.numbers.deployment_poll_seconds));
    }

    (
      Ran {
        code: 0,
        said: started.said,
      },
      Some(unit),
    )
  }
}
