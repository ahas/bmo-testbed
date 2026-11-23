//! The `release` verb (docs/deploy.md, 「The set's release」; docs/boundary.md, 「`release`」): the set read
//! from its folder and fetched, every repository's change worked out and printed, and the release made in git
//! where it is neither a dry run nor a proposal; or the step the commits since the last release propose.

use std::path::Path;

use bmo_testbed_core::answer::{Cut, cap_items};
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::versions::VERSIONS;
use bmo_testbed_core::verb::{REFUSED_EXIT_CODE, data_root};
use bmo_testbed_core::version::release_tag;
use bmo_testbed_suite::release::{
  Git, Member, MemberRead, SetRead, Standing, Step, machine_git, make_release, member_lines, proposal_line,
  propose, read_member, read_set, refusal, subjects_since_release,
};
use semver::Version;

use crate::own::called_from;
use crate::words::ReleaseArgs;

/// `release`: its code, every line said on standard output and every refusal on standard error.
pub fn release(args: &ReleaseArgs) -> u8 {
  let set = called_from().join(&args.set);

  released(args, &set, &machine_git, &data_root().join("testbed"))
}

// Whether a repository is one the caller asked to see.
fn shown(args: &ReleaseArgs, member: &Member) -> bool {
  args.repository.is_empty() || args.repository.contains(&member.name)
}

// Blocks of lines printed under the ceiling, the cut said.
fn print_capped(blocks: &[String]) {
  let said = &VERSIONS.release;
  let render = |kept: &[String]| kept.join("\n");
  let capped = cap_items(blocks, render, Cut::new(&said.cut_unit, &said.cut_how));

  if !capped.kept.is_empty() {
    println!("{}", render(capped.kept));
  }

  if let Some(note) = capped.note {
    println!("{note}");
  }
}

/// The verb over the set standing in `set`, git asked through `git`, the release's worktrees made under
/// `scratch`.
pub fn released(args: &ReleaseArgs, set: &Path, git: Git, scratch: &Path) -> u8 {
  let said = &VERSIONS.release;
  let set_text = set.to_string_lossy();

  if !set.is_dir() {
    eprintln!("{}", fill_marks(&said.no_set_folder, &[("set", &set_text)]));
    return REFUSED_EXIT_CODE;
  }

  let read = read_set(set, git);

  if read.members.is_empty() {
    eprintln!("{}", fill_marks(&said.no_set, &[("set", &set_text)]));
    return REFUSED_EXIT_CODE;
  }

  let mut tags: Vec<(String, Vec<String>)> = Vec::new();

  for member in &read.members {
    let fetched = git(&["fetch", "--quiet", "--tags", "origin"], &member.checkout)
      .and_then(|_| git(&["tag", "--list"], &member.checkout));

    match fetched {
      Ok(listed) => tags.push((member.name.clone(), listed.lines().map(str::to_owned).collect())),
      Err(why) => {
        eprintln!("{}", fill_marks(&said.fetch_failed, &[("folder", &member.folder), ("why", &why)]));
        return REFUSED_EXIT_CODE;
      }
    }
  }

  let listed: Vec<Vec<String>> = tags.iter().map(|(_, listed)| listed.clone()).collect();
  let standing = Standing::of(&listed);

  if args.propose {
    return proposed(args, &read, &standing, git);
  }

  let asked = args.version.clone().unwrap_or_default();
  let Ok(to) = Version::parse(asked.trim()) else {
    eprintln!(
      "{}",
      fill_marks(&said.refused_version, &[("version", &asked), ("last", &standing.version().to_string())])
    );
    return REFUSED_EXIT_CODE;
  };
  let tag = release_tag(&to.to_string());
  let tagged: Vec<String> = tags
    .iter()
    .filter(|(_, listed)| listed.contains(&tag))
    .map(|(name, _)| name.clone())
    .collect();
  let mut reads: Vec<MemberRead> = Vec::new();

  for member in &read.members {
    match read_member(member, &standing, &to, git) {
      Ok(one) => reads.push(one),
      Err(why) => {
        eprintln!("{}", fill_marks(&said.unread, &[("folder", &member.folder), ("why", &why)]));
        return REFUSED_EXIT_CODE;
      }
    }
  }

  println!(
    "{}",
    fill_marks(
      &said.heading,
      &[
        ("version", &to.to_string()),
        ("count", &reads.len().to_string()),
        ("set", &set_text),
        ("from", &standing.shown()),
      ],
    )
  );

  let blocks: Vec<String> = reads
    .iter()
    .filter(|one| shown(args, &one.member))
    .map(|one| member_lines(one, &to).join("\n"))
    .collect();

  print_capped(&blocks);
  print_passed_over(&read);

  if let Some(refused) = refusal(&to, &standing, &tagged, &reads) {
    eprintln!("{refused}");
    return REFUSED_EXIT_CODE;
  }

  if args.dry_run {
    println!("{}", said.dry);
    return 0;
  }

  match make_release(&reads, &to, &scratch.join(format!("release-{}", std::process::id())), git) {
    Ok(commits) => {
      println!(
        "{}",
        fill_marks(
          &said.released,
          &[
            ("version", &to.to_string()),
            ("count", &reads.len().to_string()),
            ("commits", &commits.to_string())
          ],
        )
      );
      0
    }
    Err(line) => {
      eprintln!("{line}");
      1
    }
  }
}

fn print_passed_over(read: &SetRead) {
  for one in &read.passed_over {
    println!(
      "{}",
      fill_marks(&VERSIONS.release.passed_over, &[("folder", &one.folder), ("why", &one.why)])
    );
  }
}

// The proposal over every repository, each one asked for printed, then the set's step.
fn proposed(args: &ReleaseArgs, read: &SetRead, standing: &Standing, git: Git) -> u8 {
  let said = &VERSIONS.propose;
  let mut lines: Vec<String> = Vec::new();
  let mut step = Step::Patch;

  for member in &read.members {
    let subjects = match subjects_since_release(member, git) {
      Ok(subjects) => subjects,
      Err(why) => {
        eprintln!("{}", fill_marks(&VERSIONS.release.unread, &[("folder", &member.folder), ("why", &why)]));
        return REFUSED_EXIT_CODE;
      }
    };
    let proposal = propose(&member.name, &subjects);

    if let Some((strongest, _)) = &proposal.strongest {
      step = step.max(*strongest);
    }

    if shown(args, member) {
      lines.push(proposal_line(&proposal));
    }
  }

  let since = match standing {
    Standing::Before => said.since_none.clone(),
    Standing::At(version) => release_tag(&version.to_string()),
  };

  println!(
    "{}",
    fill_marks(&said.heading, &[("since", &since), ("count", &read.members.len().to_string())])
  );
  print_capped(&lines);
  println!(
    "{}",
    fill_marks(
      &said.step,
      &[
        ("step", step.word()),
        ("from", &standing.version().to_string()),
        ("to", &step.from(standing).to_string())
      ],
    )
  );
  0
}
