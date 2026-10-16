# Working in bmo-testbed

These rules hold every change to this repository.

## The documents come first

- `docs/` is the canonical statement of what the testbed does; the code is one realisation of it.
  Before changing code, read the document covering that area.
- A change that would make a document wrong or incomplete repairs the document first, stating what is
  true now, and the code is written to match.
- `docs/boundary.md` is the public contract. A change to anything it lists follows its version rules,
  and the boundary document changes in the same commit.
- A document states what is true now: no change history, no rationale told through an earlier
  design, no pasted source, signature or config fragment, and no file-and-line pins.

## Language

- Everything in this repository is English: code, comments, strings, tests, documents, commit messages
  and branch names. No Korean in code or tests.
- Test input is neutral English. A fixture recorded from elsewhere stays as it was recorded.
- No personal data: no user names, home directories, machine names or names of any agent or product
  that uses the testbed. A path is written from `$HOME` or from a variable.

## What the testbed may know

- **No repository but its own.** The testbed knows no consumer by name or path; whatever differs per
  repository is read from that repository's `testbed.toml`. A per-repository value added anywhere
  else is misplaced.
- **No lookup table standing in for polymorphism.** A language is a member of the language type and
  answers for itself; a deploy step is a declared step; a unit says its own deploy mark.
- **Dependencies:** crates from the registry pinned in the workspace manifest, and among other
  repositories only the base libraries `bmo-chat`, `bmo-chat-client` and `bmo-ui`. The quick checks'
  dependency check holds this.
- **No hand-written parser for a general format.** TOML, JSON, YAML, markdown, shell lines and command
  lines are read by a maintained library; where none fits, the reason is written beside the code.

## Code and tests

- Tests live in each crate's `tests/` directory, not in the source files.
- The testbed's own fixed numbers and words stand in `policy/` and are read through `core`'s policy
  loader, not written into the code.
- A verb's answer is English, thin by default, with `--json` for structure, cut at 30,000 characters
  with every cut said (`docs/boundary.md`).

## The stages of a change

1. **Writing:** the only check is `cargo check --workspace --all-targets`, on the narrowest crate that
   is cheaper.
2. **Review:** read the whole diff against the document governing it, then `cargo fmt --check` and
   `cargo clippy --workspace --all-targets --locked -- -D warnings`.
3. **Verification:** the tests of the crates touched, then the whole workspace's tests once, through
   the testbed's own queue, output written to a file and the file read.

A change is not done before stage 3 has passed.
