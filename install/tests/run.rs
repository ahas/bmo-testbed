//! The install's run (docs/install.md, 「The order of a run」), against a machine of the test's own: an index with a
//! presets file, two repositories already checked out, each with its install declaration and its testbed
//! declaration, and a data root of its own.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use bmo_testbed_install::run::PORTS_FILE;
use bmo_testbed_install::run::{Outcome, STATE_FILE};
use bmo_testbed_install::{Ask, Install, Line, Machine, Mode, Ran, Services, Status};

#[derive(Default)]
struct Stand {
  missing: BTreeSet<String>,
  databases: RefCell<BTreeSet<String>>,
  deployed: RefCell<Vec<String>>,
  ran: RefCell<Vec<String>>,
  taken_port: Option<u16>,
}

impl Machine for Stand {
  fn has_program(&self, program: &str) -> bool {
    !self.missing.contains(program)
  }

  fn shell(&self, command: &str, dir: &Path, env: &[(String, String)]) -> Ran {
    self.ran.borrow_mut().push(command.to_owned());

    let output = std::process::Command::new("sh")
      .arg("-c")
      .arg(command)
      .current_dir(dir)
      .envs(env.iter().map(|(name, value)| (name, value)))
      .output()
      .unwrap();

    Ran {
      code: output.status.code().unwrap_or(-1),
      said: String::from_utf8_lossy(&output.stdout).into_owned(),
    }
  }

  fn clone_repository(&self, _url: &str, into: &Path) -> Ran {
    std::fs::create_dir_all(into.join(".git")).unwrap();
    Ran {
      code: 0,
      said: String::new(),
    }
  }

  fn fetch(&self, _checkout: &Path) -> Ran {
    Ran {
      code: 0,
      said: String::new(),
    }
  }

  fn main_head(&self, _checkout: &Path, _main: &str) -> Option<String> {
    Some("0123456789abcdef".to_owned())
  }

  fn database_exists(&self, name: &str) -> Result<bool, String> {
    Ok(self.databases.borrow().contains(name))
  }

  fn create_database(&self, name: &str) -> Ran {
    self.databases.borrow_mut().insert(name.to_owned());
    Ran {
      code: 0,
      said: String::new(),
    }
  }

  fn port_free(&self, port: u16) -> bool {
    self.taken_port != Some(port)
  }

  fn random(&self, bytes: usize) -> Result<Vec<u8>, String> {
    Ok(vec![7; bytes])
  }

  // A deployment lays the deploy tree's link and the record naming the commit, as the deploy verb does.
  fn deploy(&self, checkout: &Path, commit: &str, _message: &str) -> (Ran, Option<String>) {
    let root = checkout.join("deploy-root");

    std::fs::create_dir_all(root.join("deploy-releases/one")).unwrap();
    std::os::unix::fs::symlink(root.join("deploy-releases/one"), root.join("deploy")).unwrap();
    std::fs::write(root.join("deployed.json"), format!("{{\"commit\":\"{commit}\"}}")).unwrap();
    self
      .deployed
      .borrow_mut()
      .push(checkout.file_name().unwrap().to_string_lossy().into_owned());
    (
      Ran {
        code: 0,
        said: String::new(),
      },
      Some("bmo-deploy-1".to_owned()),
    )
  }
}

const PRESETS: &str = r#"
presets = 1

[[repository]]
name = "one"
folder = "services/one"
url = "https://example.invalid/one.git"

[[repository]]
name = "two"
folder = "services/two"
url = "https://example.invalid/two.git"
features = ["study"]

[preset.solo]
summary = "Both."
units = ["one", "two"]
features = []

[preset.group]
summary = "Both, with teams."
extends = "solo"
features = ["teams"]
"#;

const ONE: &str = r#"
install = 1
restart = "touch restarted"
report = "http://127.0.0.1:{{port.one.web}}/"

[[needs]]
program = "git"
what = "git, from the system's packages"

[[port]]
name = "one.web"
number = 4100

[[token]]
name = "one.service"
file = "env/one.token"

[[input]]
name = "one.owner"
what = "The owner's name, which the person chooses"
file = "env/one-owner"

[[input]]
name = "one.language"
what = "The language"
file = "install/one-language"
default = "en"

[[database]]
name = "one"

[[settings]]
file = "env/one.toml"
text = "token = \"{{token.one.service}}\"\nlanguage = \"{{input.one.language}}\"\nfrom = \"{{file.env/two.minted}}\"\n"

[[settings]]
file = "env/console.env"
text = "PORT={{port.one.web}}\n"

[[after]]
id = "first-account"
run = "cat \"$BMO_INPUT_ONE_OWNER\" > made-for && touch account"
done = "test -f account"
inputs = ["one.owner"]
"#;

const TWO: &str = r#"
install = 1

[[token]]
name = "two.console"
file = "env/two-console.token"
encoding = "base64"

[[line]]
file = "env/console.env"
key = "TWO_ADDRESS"
value = "http://127.0.0.1:4200 {{token.two.console}}"

[[verify]]
id = "hello"
run = "true"
what = "a message is read back"
"#;

fn testbed(checkout: &Path) -> String {
  format!(
    "testbed = 0\nname = \"x\"\n\n[deploy]\nroot = \"{}\"\ndata_root = \"{}\"\n",
    checkout.join("deploy-root").display(),
    checkout.join("data").display()
  )
}

struct Ground {
  _dir: tempfile::TempDir,
  index: PathBuf,
  data_root: PathBuf,
  home: PathBuf,
}

fn ground() -> Ground {
  let dir = tempfile::tempdir().unwrap();
  let index = dir.path().join("index");
  let data_root = dir.path().join("data");
  let home = dir.path().join("home");

  std::fs::create_dir_all(&index).unwrap();
  std::fs::write(index.join("presets.toml"), PRESETS).unwrap();

  for (name, declaration) in [("one", ONE), ("two", TWO)] {
    let checkout = index.join("services").join(name);

    std::fs::create_dir_all(checkout.join(".git")).unwrap();
    std::fs::write(checkout.join("install.toml"), declaration).unwrap();
    std::fs::write(checkout.join("testbed.toml"), testbed(&checkout)).unwrap();
  }

  Ground {
    index,
    data_root,
    home,
    _dir: dir,
  }
}

fn ask(ground: &Ground, mode: Mode, dry_run: bool) -> Ask {
  Ask {
    mode,
    index: Some(ground.index.clone()),
    data_root: ground.data_root.clone(),
    home: ground.home.clone(),
    user: "tester".to_owned(),
    dry_run,
    cwd: ground.index.clone(),
    prefix: ground.index.clone(),
    services: Services::default(),
  }
}

fn solo() -> Mode {
  Mode::Preset {
    name: "solo".to_owned(),
    with: Vec::new(),
  }
}

fn run(ground: &Ground, machine: &Stand, mode: Mode, dry_run: bool) -> (Outcome, Vec<Line>) {
  let mut lines = Vec::new();
  let mut say = |line: &Line| lines.push(line.clone());
  let outcome = Install::new(ask(ground, mode, dry_run), machine, &mut say).run();

  (outcome, lines)
}

fn status_of<'a>(lines: &'a [Line], step: &str) -> Option<&'a Line> {
  lines.iter().find(|line| line.step == step)
}

#[test]
fn a_required_input_not_given_stops_the_run_before_anything_is_written() {
  let ground = ground();
  let machine = Stand::default();
  let (outcome, lines) = run(&ground, &machine, solo(), false);

  assert_eq!(outcome, Outcome::Waiting);

  let needed = status_of(&lines, "input:one.owner").unwrap();

  assert_eq!(needed.status, Status::NeedsInput);
  assert_eq!(needed.detail, "The owner's name, which the person chooses");
  assert_eq!(needed.next, "bmo install --set one.owner");
  assert_eq!(status_of(&lines, "input:one.language").unwrap().status, Status::Ok);
  assert!(!ground.data_root.join("env/one.token").exists());
  assert!(machine.deployed.borrow().is_empty());
  assert_eq!(lines.last().unwrap().step, "done");
  assert_eq!(lines.last().unwrap().status, Status::NeedsInput);
}

#[test]
fn a_dry_run_says_the_whole_plan_and_changes_nothing() {
  let ground = ground();
  let machine = Stand::default();
  let (outcome, lines) = run(&ground, &machine, solo(), true);

  // The input is still asked for; everything else that would act says planned.
  assert_eq!(outcome, Outcome::Waiting);
  assert_eq!(status_of(&lines, "token:one.service").unwrap().status, Status::Planned);
  assert_eq!(status_of(&lines, "database:one").unwrap().status, Status::Planned);
  assert_eq!(status_of(&lines, "deploy:one").unwrap().status, Status::Planned);
  assert_eq!(status_of(&lines, "verify:two:hello").unwrap().status, Status::Planned);
  assert!(!ground.data_root.exists());
  assert!(machine.deployed.borrow().is_empty());
  assert!(machine.databases.borrow().is_empty());
}

#[test]
fn a_whole_run_makes_writes_deploys_and_continues_where_it_stopped() {
  let ground = ground();
  let machine = Stand::default();

  std::fs::create_dir_all(ground.data_root.join("env")).unwrap();
  std::fs::write(ground.data_root.join("env/one-owner"), "owner\n").unwrap();

  let (outcome, lines) = run(&ground, &machine, solo(), false);
  let printed: Vec<String> = lines.iter().map(Line::json).collect();

  // The settings file naming a file the second repository makes waits, and nothing makes it, so it fails at the
  // end; everything before stands.
  assert_eq!(outcome, Outcome::Failed, "{printed:#?}");
  assert_eq!(
    std::fs::read_to_string(ground.data_root.join("env/one.token")).unwrap(),
    format!("{}\n", "07".repeat(32))
  );
  assert_eq!(
    std::fs::read_to_string(ground.data_root.join("env/two-console.token"))
      .unwrap()
      .trim()
      .len(),
    44
  );
  assert!(machine.databases.borrow().contains("one"));
  assert_eq!(*machine.deployed.borrow(), ["one", "two"]);
  assert_eq!(std::fs::read_to_string(ground.index.join("services/one/made-for")).unwrap(), "owner\n");

  let console = std::fs::read_to_string(ground.data_root.join("env/console.env")).unwrap();

  assert!(console.starts_with("PORT=4100\nTWO_ADDRESS=http://127.0.0.1:4200 "), "{console}");
  assert_eq!(
    std::fs::read_to_string(ground.data_root.join("env/console-features.toml"))
      .unwrap()
      .lines()
      .skip(1)
      .collect::<Vec<_>>(),
    ["study = true", "teams = false"]
  );
  assert!(ground.data_root.join(STATE_FILE).exists());

  // The file the settings wait for stands now: the next run writes them, restarts the repository already live,
  // and deploys and makes nothing twice.
  std::fs::write(ground.data_root.join("env/two.minted"), "minted\n").unwrap();

  let (outcome, lines) = run(&ground, &machine, solo(), false);
  let printed: Vec<String> = lines.iter().map(Line::json).collect();

  assert_eq!(outcome, Outcome::Done, "{printed:#?}");
  assert_eq!(*machine.deployed.borrow(), ["one", "two"]);
  assert!(
    std::fs::read_to_string(ground.data_root.join("env/one.toml"))
      .unwrap()
      .contains("from = \"minted\"")
  );
  assert!(ground.index.join("services/one/restarted").exists());
  assert_eq!(
    status_of(&lines, "after:one:first-account").unwrap().detail,
    "first-account already stands"
  );
  assert_eq!(status_of(&lines, "deploy:one").unwrap().detail, "one is live already");
  assert_eq!(console, std::fs::read_to_string(ground.data_root.join("env/console.env")).unwrap());

  let done = lines.last().unwrap();

  assert_eq!(done.status, Status::Ok);
  assert_eq!(done.next, "open http://127.0.0.1:4100/");
}

#[test]
fn a_missing_program_stops_the_run_naming_it() {
  let ground = ground();
  let machine = Stand {
    missing: ["git".to_owned()].into(),
    ..Stand::default()
  };
  let (outcome, lines) = run(&ground, &machine, solo(), false);
  let missing = status_of(&lines, "needs:git").unwrap();

  assert_eq!(outcome, Outcome::Failed);
  assert_eq!(missing.status, Status::Failed);
  assert!(missing.detail.contains("git, from the system's packages"));
  assert!(status_of(&lines, "input:one.owner").is_none());
}

#[test]
fn a_port_another_program_holds_moves_to_the_next_free_one_which_is_kept() {
  let ground = ground();
  let machine = Stand {
    taken_port: Some(4100),
    ..Stand::default()
  };

  std::fs::create_dir_all(ground.data_root.join("env")).unwrap();
  std::fs::write(ground.data_root.join("env/one-owner"), "owner\n").unwrap();
  std::fs::write(ground.data_root.join("env/two.minted"), "minted\n").unwrap();

  let (_, lines) = run(&ground, &machine, solo(), false);
  let moved = status_of(&lines, "port:one.web").unwrap();

  assert_eq!(moved.status, Status::Ok);
  assert_eq!(moved.detail, "one.web listens on 4101, since 4100 is taken on this machine");
  assert_eq!(
    std::fs::read_to_string(ground.data_root.join("env/console.env"))
      .unwrap()
      .lines()
      .next(),
    Some("PORT=4101")
  );
  assert_eq!(
    std::fs::read_to_string(ground.data_root.join(PORTS_FILE))
      .unwrap()
      .replace(char::is_whitespace, ""),
    "{\"one.web\":4101}"
  );

  // The next run reads the number kept, whatever the machine answers now.
  let (_, lines) = run(&ground, &Stand::default(), solo(), false);

  assert_eq!(
    status_of(&lines, "port:one.web").unwrap().detail,
    "one.web listens on 4101, as the install chose before"
  );
}

#[test]
fn a_repository_placed_outside_the_prefix_is_said() {
  let ground = ground();
  let mut asked = ask(&ground, solo(), true);

  asked.prefix = ground.home.join("elsewhere");

  let mut lines = Vec::new();
  let mut say = |line: &Line| lines.push(line.clone());
  let _ = Install::new(asked, &Stand::default(), &mut say).run();

  assert_eq!(status_of(&lines, "place:one").unwrap().status, Status::Failed);
}

#[test]
fn a_repository_with_no_install_declaration_fails() {
  let ground = ground();

  std::fs::remove_file(ground.index.join("services/two/install.toml")).unwrap();

  let (outcome, lines) = run(&ground, &Stand::default(), solo(), false);

  assert_eq!(outcome, Outcome::Failed);
  assert_eq!(status_of(&lines, "declaration:two").unwrap().status, Status::Failed);
}

#[test]
fn a_repository_not_checked_out_is_cloned() {
  let ground = ground();

  std::fs::remove_dir_all(ground.index.join("services/two")).unwrap();

  let (_, lines) = run(&ground, &Stand::default(), solo(), false);

  assert_eq!(
    status_of(&lines, "clone:two").unwrap().detail.split(" cloned from ").next(),
    Some("two")
  );
}

#[test]
fn an_input_is_set_from_standard_input_where_its_declaration_keeps_it() {
  // The value is read from the process's standard input, which a test cannot hand; the refusal for an input no
  // declaration names stands for the reading.
  let ground = ground();
  let (outcome, _) = run(&ground, &Stand::default(), Mode::Set("ghost".to_owned()), false);

  assert!(matches!(outcome, Outcome::Refused(why) if why.contains("ghost")));
}

#[test]
fn adding_needs_an_install_to_add_to() {
  let ground = ground();
  let (outcome, _) = run(&ground, &Stand::default(), Mode::Add("two".to_owned()), false);

  assert!(matches!(outcome, Outcome::Refused(why) if why.contains("holds no install")));
}

#[test]
fn the_status_names_each_repository_and_whether_it_is_live() {
  let ground = ground();
  let machine = Stand::default();

  std::fs::create_dir_all(ground.data_root.join("env")).unwrap();
  std::fs::write(ground.data_root.join("env/one-owner"), "owner\n").unwrap();
  std::fs::write(ground.data_root.join("env/two.minted"), "minted\n").unwrap();
  assert_eq!(run(&ground, &machine, solo(), false).0, Outcome::Done);

  let (outcome, lines) = run(&ground, &machine, Mode::Status, false);

  assert_eq!(outcome, Outcome::Done);
  assert!(status_of(&lines, "status:one").unwrap().detail.contains("live 0123456789abcdef"));
  assert_eq!(
    status_of(&lines, "port:one.web").unwrap().detail,
    "one.web listens on 4100, as the install chose before"
  );
}
