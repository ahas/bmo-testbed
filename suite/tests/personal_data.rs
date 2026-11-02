//! The repository's personal-data prohibition (`docs/PERSONAL-DATA.md`): no tracked file holds a mail
//! address outside the reserved example domains, a person's home path, a telephone number, a token or a
//! private network address. The neutral forms an example writes are not findings. The check carries the
//! patterns alone; no list of addresses, paths or values stands in the repository.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use regex::Regex;

/// The home folders a neutral example writes, so a person's own is not among them.
const NEUTRAL_HOME: [&str; 5] = ["user", "tester", "qa-admin", "placeholder", "example"];

/// The patterns that read a line for each kind of personal data.
struct Rules {
  mail: Regex,
  home: Regex,
  telephone: Regex,
  token: Regex,
  private: Regex,
}

impl Rules {
  fn read() -> Self {
    Self {
      mail: Regex::new(concat!(r"\b[A-Za-z0-9._%+-]+@", r"[A-Za-z][A-Za-z0-9.-]*\.[A-Za-z]{2,}\b"))
        .expect("the mail pattern reads"),
      home: Regex::new(r"(?:/home/|/Users/|[A-Za-z]:\\Users\\)([A-Za-z0-9._-]+)")
        .expect("the home path pattern reads"),
      telephone: Regex::new(concat!(
        r"\b01[016789][-. ]?\d{3,4}[-. ]?\d{4}\b|",
        r"\+\d{1,3}[-. ]?\d{2,4}[-. ]?\d{3,4}[-. ]?\d{4}\b|",
        r"\+\d{9,15}\b"
      ))
      .expect("the telephone pattern reads"),
      token: Regex::new(concat!(
        r"gh[pousr]_[A-Za-z0-9]{20,}|",
        r"github_pat_[A-Za-z0-9_]{20,}|",
        r"sk-[A-Za-z0-9_-]{20,}|",
        r"xox[baprs]-[A-Za-z0-9-]{10,}|",
        r"AKIA[0-9A-Z]{16}|",
        r"AIza[0-9A-Za-z_-]{35}"
      ))
      .expect("the token pattern reads"),
      private: Regex::new(concat!(
        r"(?i)\b(?:10\.\d{1,3}\.\d{1,3}\.\d{1,3}|",
        r"192\.168\.\d{1,3}\.\d{1,3}|",
        r"172\.(?:1[6-9]|2\d|3[01])\.\d{1,3}\.\d{1,3})\b|",
        r"\b(?:f[cd][0-9a-f]{2}|fe80):"
      ))
      .expect("the private address pattern reads"),
    }
  }

  /// The kinds the line holds, each once, in a fixed order and with a neutral example left out.
  fn kinds(&self, line: &str) -> Vec<&'static str> {
    let mut kinds = Vec::new();

    if self.mail.find_iter(line).any(|address| !is_example_address(address.as_str())) {
      kinds.push("mail address");
    }
    if self
      .home
      .captures_iter(line)
      .any(|capture| capture.get(1).is_none_or(|folder| !is_neutral_home(folder.as_str())))
    {
      kinds.push("home path");
    }
    if self.telephone.is_match(line) {
      kinds.push("telephone number");
    }
    if self.token.is_match(line) {
      kinds.push("token");
    }
    if self.private.is_match(line) {
      kinds.push("private network address");
    }

    kinds
  }
}

/// Whether a mail address stands in the reserved example space, where a neutral example writes one.
fn is_example_address(address: &str) -> bool {
  let Some((_, domain)) = address.rsplit_once('@') else {
    return false;
  };
  let domain = domain.to_lowercase();

  ["example.com", "example.net", "example.org"]
    .iter()
    .any(|example| domain == *example || domain.ends_with(&format!(".{example}")))
    || [".example", ".test", ".invalid", ".localhost"]
      .iter()
      .any(|suffix| domain.ends_with(suffix))
}

/// Whether a home path's own folder is a neutral example rather than a person's.
fn is_neutral_home(folder: &str) -> bool {
  NEUTRAL_HOME.contains(&folder) || folder.starts_with("agent-")
}

/// The repository root: the first folder above this crate that holds git's own folder.
fn root() -> PathBuf {
  Path::new(env!("CARGO_MANIFEST_DIR"))
    .ancestors()
    .find(|folder| folder.join(".git").exists())
    .expect("the repository root stands above the crate")
    .to_owned()
}

/// Every file git tracks or would track: the committed ones and the new ones it does not ignore.
fn tracked(root: &Path) -> Vec<String> {
  let output = Command::new("git")
    .args(["ls-files", "--cached", "--others", "--exclude-standard", "-z"])
    .current_dir(root)
    .stdin(Stdio::null())
    .output()
    .expect("git lists the files");

  assert!(
    output.status.success(),
    "git ls-files failed: {}",
    String::from_utf8_lossy(&output.stderr)
  );

  String::from_utf8(output.stdout)
    .expect("the paths are text")
    .split('\0')
    .filter(|path| !path.is_empty() && root.join(path).is_file())
    .map(str::to_owned)
    .collect()
}

/// Each finding in a text, as the failure reports it: the file, the line and the kind, never the value.
fn findings(path: &str, text: &str, rules: &Rules) -> Vec<String> {
  text
    .lines()
    .enumerate()
    .flat_map(|(index, line)| {
      rules.kinds(line).into_iter().map(move |kind| format!("{path}:{}: {kind}", index + 1))
    })
    .collect()
}

#[test]
fn no_tracked_file_holds_personal_data() {
  let root = root();
  let rules = Rules::read();
  let found: Vec<String> = tracked(&root)
    .into_iter()
    .filter_map(|path| std::fs::read_to_string(root.join(&path)).ok().map(|text| (path, text)))
    .flat_map(|(path, text)| findings(&path, &text, &rules))
    .collect();

  assert!(found.is_empty(), "personal data in tracked files:\n{}", found.join("\n"));
}

#[test]
fn the_check_names_the_kind_and_the_place_and_never_the_value() {
  let rules = Rules::read();
  let mail = ["someone", "@", "corp", ".zz"].concat();
  let home = ["/home/", "alice"].concat();
  let telephone = ["+82", "10", "1234", "5678"].concat();
  let token = ["gh", "p_", &"A".repeat(24)].concat();
  let private = ["192", ".168.", "10", ".20"].concat();
  let text = format!("one\n{mail} and {home}\n{telephone}\n{token}\n{private}\n");
  let found = findings("notes.txt", &text, &rules);

  assert_eq!(
    found,
    [
      "notes.txt:2: mail address",
      "notes.txt:2: home path",
      "notes.txt:3: telephone number",
      "notes.txt:4: token",
      "notes.txt:5: private network address",
    ]
  );
  assert!(!found.iter().any(|line| line.contains("someone") || line.contains("alice")));
}
