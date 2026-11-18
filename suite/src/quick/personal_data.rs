//! The personal-data check over every file of the checkout (docs/checks.md, 「The quick checks」): a line
//! holding one of the forms the policy names is refused where it stands, with the file, the line and the
//! kind and never the value. What a form lets stand is the policy's and the declaration's: an address in the
//! space kept for documents, a home folder named for a role, an address of no private network, and whatever
//! the repository's declaration allows by name.

use std::path::Path;
use std::sync::LazyLock;

use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::quick_checks::{PersonalDataWords, QUICK_CHECKS};
use regex::{Regex, RegexSet};

use super::Findings;
use super::files::read_text;
use crate::files::checkout_files;

// One form: the kind a finding names and the pattern a line is read for.
struct Form {
  kind: &'static str,
  pattern: Regex,
}

struct Forms {
  // Every pattern at once, so a line holding none is passed with one read.
  any: RegexSet,
  each: Vec<Form>,
}

static FORMS: LazyLock<Forms> = LazyLock::new(|| {
  let stated = &QUICK_CHECKS.personal_data.forms;

  Forms {
    any: RegexSet::new(stated.iter().map(|form| form.pattern.as_str())).expect("the policy's forms read"),
    each: stated
      .iter()
      .map(|form| Form {
        kind: form.kind.as_str(),
        pattern: Regex::new(&form.pattern).expect("a policy form reads"),
      })
      .collect(),
  }
});

// Whether a mail address stands in the space kept for documents rather than naming a person.
fn is_documentation_address(words: &PersonalDataWords, address: &str) -> bool {
  let Some((_, domain)) = address.rsplit_once('@') else { return false };
  let domain = domain.trim_end_matches('.').to_ascii_lowercase();

  words
    .documentation_domains
    .iter()
    .any(|base| domain == *base || domain.ends_with(&format!(".{base}")))
    || words.documentation_suffixes.iter().any(|suffix| domain.ends_with(suffix.as_str()))
}

// Whether a home path names a folder of a role rather than a person's.
fn is_neutral_home(words: &PersonalDataWords, path: &str) -> bool {
  let folder = path.rsplit(['/', '\\']).next().unwrap_or_default();

  words.neutral_homes.iter().any(|neutral| neutral == folder)
}

// Whether an address is of a private network: the IPv4 blocks kept for one, the shared and link-local blocks,
// and every IPv6 address the forms reach, which are already narrowed to the private prefixes.
fn is_private_address(address: &str) -> bool {
  if address.contains(':') {
    return true;
  }

  let octets: Vec<u8> = address.split('.').filter_map(|part| part.parse().ok()).collect();
  let [first, second, ..] = octets.as_slice() else { return false };

  octets.len() == 4
    && (*first == 10
      || (*first == 172 && (16..=31).contains(second))
      || (*first == 192 && *second == 168)
      || (*first == 169 && *second == 254)
      || (*first == 100 && (64..=127).contains(second)))
}

// Whether a mail-shaped match names a unit of the service manager, written as a template and its instance,
// rather than a mailbox.
fn is_unit_name(words: &PersonalDataWords, found: &str) -> bool {
  words.unit_suffixes.iter().any(|suffix| found.ends_with(suffix.as_str()))
}

// Whether a match stands inside a longer number, as the digits of a fraction do: a digit or a point touches
// it on either side.
fn is_part_of_a_number(line: &str, found: &regex::Match<'_>) -> bool {
  let before = line[..found.start()].chars().next_back();
  let mut after = line[found.end()..].chars();
  let next = after.next();

  before.is_some_and(|one| one.is_ascii_digit() || one == '.')
    || next.is_some_and(|one| one.is_ascii_digit())
    || (next == Some('.') && after.next().is_some_and(|one| one.is_ascii_digit()))
}

// Whether what a form found is a finding: not what the policy lets stand for its kind, nor anything the
// declaration allows.
fn kept(
  words: &PersonalDataWords,
  allowed: &[String],
  kind: &str,
  line: &str,
  at: &regex::Match<'_>,
) -> bool {
  let found = at.as_str();

  if allowed.iter().any(|allowed| !allowed.is_empty() && found.contains(allowed.as_str())) {
    return false;
  }

  if kind == words.mail_kind {
    !is_documentation_address(words, found) && !is_unit_name(words, found)
  } else if kind == words.telephone_kind {
    !is_part_of_a_number(line, at)
  } else if kind == words.home_kind {
    !is_neutral_home(words, found)
  } else if kind == words.network_kind {
    is_private_address(found)
  } else {
    true
  }
}

/// The kinds each line of a text holds, as findings naming `path`, the line and the kind.
pub fn personal_data_in(path: &str, text: &str, allowed: &[String]) -> Vec<String> {
  let words = &QUICK_CHECKS.personal_data;
  let forms = &*FORMS;
  let mut findings = Vec::new();

  for (number, line) in text.lines().enumerate() {
    if !forms.any.is_match(line) {
      continue;
    }

    let mut kinds: Vec<&str> = Vec::new();

    for form in &forms.each {
      if !kinds.contains(&form.kind)
        && form.pattern.find_iter(line).any(|found| kept(words, allowed, form.kind, line, &found))
      {
        kinds.push(form.kind);
      }
    }

    findings.extend(kinds.into_iter().map(|kind| {
      fill_marks(&words.hit, &[("path", path), ("line", &(number + 1).to_string()), ("kind", kind)])
    }));
  }

  findings
}

/// The check over every file of the checkout at `root` that stands in the tree.
pub fn checkout_personal_data(root: &Path, allowed: &[String]) -> Findings {
  let files: Vec<String> =
    checkout_files(root).into_iter().filter(|path| root.join(path).is_file()).collect();
  let findings = files
    .iter()
    .flat_map(|path| personal_data_in(path, &read_text(root, path), allowed))
    .collect();

  Findings {
    paths: files.len(),
    findings,
  }
}
