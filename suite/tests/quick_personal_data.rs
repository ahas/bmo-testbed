//! The personal-data check (docs/checks.md, 「The quick checks」): each form found on its line and named by
//! its kind, and what the policy and a declaration let stand. Every sample is put together from parts, so
//! this file holds none of what the check looks for.

use bmo_testbed_suite::quick::personal_data::personal_data_in;

fn kinds_in(line: &str, allowed: &[&str]) -> Vec<String> {
  let allowed: Vec<String> = allowed.iter().map(|one| (*one).to_owned()).collect();

  personal_data_in("held.txt", &format!("a line before\n{line}\n"), &allowed)
}

fn joined(parts: &[&str]) -> String {
  parts.concat()
}

// "a line at a time for the forms the policy names ... a finding names the file, the line and the kind,
// never the value"
#[test]
fn each_form_is_found_on_its_line_and_named_by_its_kind_without_its_value() {
  let samples = [
    (joined(&["write to qa", "@", "mail", ".", "dev"]), "a mail address"),
    (joined(&["stands under ", "/", "home", "/", "somebody", "/work"]), "a home path"),
    (joined(&["stands under C", ":", "\\", "Users", "\\", "somebody"]), "a home path"),
    (joined(&["call ", "010", "-1234", "-5678"]), "a telephone number"),
    (joined(&["key ", "gh", "p_", &"aB1".repeat(8)]), "a token"),
    (joined(&["-----BEGIN ", "RSA ", "PRIVATE KEY", "-----"]), "a token"),
    (joined(&["Authorization: ", "Bearer ", &"aB1".repeat(8)]), "a token"),
    (joined(&["host ", "192", ".", "168", ".0.7"]), "a private network address"),
    (joined(&["host ", "192", ".", "168", ".0.7", "/24"]), "a private network address"),
    (joined(&["host ", "fd", "12", ":", "3456::1"]), "a private network address"),
  ];

  for (line, kind) in samples {
    assert_eq!(kinds_in(&line, &[]), [format!("held.txt:2: {kind}")], "{kind}");
  }
}

// "what stands: a mail address in the space kept for documents; a home path whose folder names a role; an
// address of no private network, the loopback among them"
#[test]
fn a_documentation_address_a_role_s_home_and_a_public_or_loopback_address_stand() {
  let standing = [
    joined(&["write to someone", "@", "example", ".", "com"]),
    joined(&["write to someone", "@", "service", ".", "test"]),
    joined(&["stands under ", "/", "home", "/", "user", "/work"]),
    joined(&["listens on ", "127", ".0.0.1"]),
    joined(&["version ", "1", ".", "2", ".", "3", ".", "4"]),
    joined(&["asks ", "8", ".8.8.8"]),
    joined(&["under user", "@", "1000", ".", "service", "/app.slice"]),
    joined(&["share 0", ".", "032", "9101", "5625"]),
    joined(&["the path ", "/", "home", "/", "someone", "/.local"]),
    joined(&["reads /layout", "/", "home", "/", "pane"]),
    joined(&["refuses ", "10", ".0.0.0", "/8 and ", "192", ".", "168", ".0.0", "/16"]),
    joined(&["refuses ", "fc", "00", "::", "/7"]),
    joined(&["key ", "gh", "p_", &"a1".repeat(12)]),
    joined(&["Authorization: ", "Bearer ", &"a1".repeat(12)]),
    joined(&["patches/some-package", "@", "2", ".", "10", ".4.patch"]),
    joined(&["some-package==", "10", ".4.0.35"]),
  ];

  for line in standing {
    assert!(kinds_in(&line, &[]).is_empty(), "{line}");
  }
}

// "a finding whose matched text holds a string the declaration's personal_data_allowed names"
#[test]
fn what_a_declaration_allows_stands_and_nothing_else_of_its_kind_does() {
  let own = joined(&["/", "home", "/", "keeper"]);
  let other = joined(&["/", "home", "/", "somebody"]);

  assert!(kinds_in(&format!("stands under {own}/work"), &[&own]).is_empty());
  assert_eq!(kinds_in(&format!("stands under {other}/work"), &[&own]), ["held.txt:2: a home path"]);
}
