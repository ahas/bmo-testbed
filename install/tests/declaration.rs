//! The install declaration (docs/install.md, 「The install declaration」).

use bmo_testbed_install::declaration::{Encoding, InstallDeclaration, Read};

#[test]
fn a_declaration_reads_every_table() {
  let declaration = InstallDeclaration::parse(
    r#"
install = 1
restart = "true"
report = "http://127.0.0.1:{{port.a.web}}/"

[[needs]]
program = "git"
what = "git, from the system's packages"

[[port]]
name = "a.web"
number = 4100

[[token]]
name = "a.service"
file = "env/a.token"
encoding = "base64"

[[input]]
name = "a.bot"
what = "The bot's token"
file = "env/a-bot"
secret = true

[[value]]
name = "a.address"
value = "http://127.0.0.1:{{port.a.web}}"

[[database]]
name = "a"

[[settings]]
file = "env/a.toml"
text = "token = \"{{token.a.service}}\"\n"

[[line]]
file = "env/console.env"
key = "A_ADDRESS"
value = "{{value.a.address}}"

[[after]]
id = "first"
run = "true"
done = "false"
inputs = ["a.bot"]

[[verify]]
id = "hello"
run = "true"
what = "a message is read back"
"#,
  )
  .unwrap();

  assert_eq!(declaration.token[0].encoding, Encoding::Base64);
  assert!(declaration.input[0].secret);
  assert_eq!(declaration.names(), ["token.a.service", "port.a.web", "input.a.bot", "value.a.address"]);
}

#[test]
fn a_key_the_schema_does_not_name_is_refused() {
  assert!(
    InstallDeclaration::parse("install = 1\n[[port]]\nname = \"a\"\nnumber = 1\nhost = \"x\"\n").is_err()
  );
}

#[test]
fn another_major_is_refused() {
  assert!(InstallDeclaration::parse("install = 2\n").unwrap_err().contains("major"));
}

#[test]
fn a_checkout_without_a_declaration_reads_as_missing() {
  let dir = tempfile::tempdir().unwrap();

  assert!(matches!(InstallDeclaration::read(dir.path()), Read::Missing));
}
