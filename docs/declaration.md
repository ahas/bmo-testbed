# The declaration: `testbed.toml`

- **One file at a repository's root,** read by every verb run inside that repository's checkout. It is
  where everything that differs between repositories stands: how the repository is tested, checked,
  built and deployed, and where its events go. A deployment runs by the declaration the commit it puts
  live carries, read from that commit (`docs/deploy.md`).
- **The root is the checkout's top level,** as git names it. A verb run outside any checkout, or in one
  with no declaration, uses the defaults below, which test and check nothing a repository did not
  declare and deploy nothing.
- **It is TOML,** read by a TOML library against this schema. A key the schema does not name is
  refused, naming the key and its table, so a misspelled key never reads as a default. A declaration
  that does not read refuses the verb with exit code 2.
- **Paths are relative to the root** unless they begin with `~/` or `/`. Globs are git's pathspec
  globs. A command is one string run through `sh -c` from the directory its table says, the root
  unless named.
- **What keys ask of each other is held as the file is read,** and a declaration breaking it is
  refused naming the key: a language or a quick check the testbed has no member for, a quick check
  without the input it reads, an exception to the shared versions naming neither a line nor a version or
  with no reason, a format toolchain that is not one name, a deploy root without its data root, a test
  database naming only one of its migrations and its migrate command, a silence limit of no seconds, and
  two steps of one table under the same id.
  Whether a path exists is asked by the verb that reads it, not here.

## The top level

| Key | Type | Default | Meaning |
|---|---|---|---|
| `testbed` | integer | required | The major of the declaration's schema this declaration is written for, which the testbed's policy holds and which moves only with a change to the declaration a declaration written before it cannot read: a key removed, or one whose meaning changes. A testbed reading another major refuses it, naming both. The set's version moves it not at all (`docs/boundary.md`, 「Versions」) |
| `name` | string | the root directory's name | The repository's name, used in unit names, backup names and events |
| `upstreams` | list of strings | `[]` | The repositories this one builds on, by their declared names. A major change to one of them runs this repository's suite through `dependents` (`docs/suite.md`, 「The dependents of a major change」). `[check] upstreams` names the same list beside the dependency check, and a declaration naming both adds them |

## `[test]`: how the suite runs

| Key | Type | Default | Meaning |
|---|---|---|---|
| `languages` | list of strings | `[]` | The kinds of unit the suite gathers: `bun` (test files run by `bun test`), `node` (test files run by `node --test`), `cargo` (crates of the Cargo workspace), `browser` (test files in the browser lane) |
| `files` | list of globs | each declared language's own naming | The test files of the file languages, each run from the folder of its runner's configuration (`docs/suite.md`, 「Which files run」) |
| `browser` | list of globs | `[]` | Test files run one at a time in the browser lane |
| `guard_seconds` | integer | none | Read and given no effect: every unit of a run is started and runs to its end |
| `file_guard_seconds` | integer | none | Read and given no effect: a unit runs until it ends, or until it neither writes nor works for the silence limit (`docs/testbed.md`) |
| `test_timeout_seconds` | integer | 120, the declaration policy's | How long one test may run before it fails: handed to a runner that takes a per-test limit, and kept by the suite over a crate's tests (`docs/suite.md`, 「How long a unit and a test may run」). At least 1 |
| `silence_seconds` | integer | 600, the lane policy's | How long a command run for the repository may write nothing before it is ended as hung: a run, a check's step, a suite's unit, the Cargo verb, a deployment's step, and a placed run on the machine it runs on (`docs/testbed.md`, 「A holder whose command falls silent lets go」). At least 1 |
| `memory_gb` | integer | 12 | The fence's memory ceiling, which `BMO_SUITE_MEMORY_GB` overrides |
| `setup` | list of commands | `[]` | Run in order to set up a fresh tree: the checkout a deployment's gate reads, a release, the daily whole run's tree, a bench copy on another machine |
| `sibling_trees` | list of paths | `[]` | The trees outside the repository its tests read by a path no manifest names, such as a file a test opens in a neighbouring checkout, each a path from the repository's root. A bench on a placed machine lays each beside it as it lays a tree the workspace reads by a path (`docs/boundary.md`, 「The placed machine」); a run here reads them where they stand |

### `[test.database]`: the test database

Present only where the repository's tests use PostgreSQL (`docs/suite.md`, 「The test database」).
The migrations and the migrate command are named together or not at all: a repository whose tests
make their own databases names neither, and its tests are handed the server alone.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `migrations` | list of globs | `[]` | The files whose contents name the template; any change makes another template |
| `migrate` | command | none | Makes the template: run with `BMO_TEST_TEMPLATE_URL` naming the empty template database, ends 0 once it is migrated |
| `max_files` | integer | 10 | How many files using the database run at once |

## `[projects]`: projects of their own

| Key | Type | Default | Meaning |
|---|---|---|---|
| `discover` | boolean | `true` | Find every directory carrying its own package manifest and a declared language's lockfile that the root workspace does not list, one inside another project included |
| `paths` | list of paths | `[]` | Projects named outright, beside or instead of the ones found |
| `check_script` | string | `check` | The manifest script that is a project's check, run by the first declared language that runs package scripts |

## `[tools]`: tools checked by their manifest

| Key | Type | Default | Meaning |
|---|---|---|---|
| `dir` | path | none | The directory whose folders are tools, each declared by its own manifest |
| `manifest` | string | `tool.toml` | The manifest's file name in a tool folder |
| `check_key` | string | `tool.check` | The dotted key in the manifest naming the folder's check command |

## `[check]`: the static check and the quick checks

| Key | Type | Default | Meaning |
|---|---|---|---|
| `quick` | list of strings | `[]` | The quick checks run: any of `wording`, `dashes`, `language` (or `korean`, the same check), `layout`, `dependencies`, `cargo-profile`, `shared-versions`, `personal-data` |
| `wording_document` | path | none | The document holding the banned terms and their allowed uses; required by `wording` |
| `wording_files` | list of globs | every file | The files `wording` holds to the document |
| `personal_data_allowed` | list of strings | `[]` | What `personal-data` lets stand in this repository: a finding whose matched text holds one of these is none |
| `people_text` | list of globs | `[]` | The files whose literals are text for a person; read by `dashes` |
| `language_ledger` | path | none | The ledger of lines holding letters that are not English allowed per code file; required by `language`. `korean_ledger` names the same key, and a declaration naming both is refused |
| `base_libraries` | list of strings | `[]` | The dependencies from other repositories the `dependencies` check allows beside registry ones |
| `upstreams` | list of strings | `[]` | The same upstream repositories as the top level, for a declaration that keeps its dependency words here; both lists are read |
| `format_toolchain` | string | none | The Rust toolchain, as rustup names it, such as `nightly`, that the Cargo workspace's formatting runs on, for a `rustfmt.toml` whose options only that toolchain's rustfmt knows; none runs it on the account's default toolchain. One name, written without the leading `+` (`docs/checks.md`, 「The static check」) |
| `main` | string | `main` | The main branch, the default base of a change, and in the repository holding the shared versions the branch the `shared-versions` check reads them from; its head is read as 「The main branch's head」 below says |

### The main branch's head

Wherever the testbed reads a repository at the head of the main branch `main` names, the committed trees
a deployment lays (`docs/deploy.md`, 「The releases」), the tree the `full` verb runs in and the file the
`shared-versions` check reads, it reads the one head this section defines.

- **Main is the branch as the repository's remote holds it.** A job's close pushes its commits onto the
  remote's main branch from whichever checkout the job stands in, and moves no checkout's own branch of
  that name, so a checkout's local branch can stand behind main for as long as its checkout works on
  another branch. The remote is the one the local branch's `remote` setting in git names, else `origin`,
  and what the machine knows of the remote's main is its remote-tracking branch under that remote's name.
- **Of the local branch and the remote-tracking branch, the one that holds the other is the head;** the
  two at one commit are that commit, and where only one of them stands it is the head. Where neither
  stands, the main branch does not stand.
- **Two that have parted,** each holding commits the other lacks, give no head either: the main branch
  does not stand, and the reader's refusal names both commits.
- **A deployment and the `full` verb ask the remote first,** fetching its main branch into the
  remote-tracking branch with no prompt for credentials, and a transfer that stalls for the policy's limit
  is ended. A remote that cannot be asked leaves the remote-tracking branch as it was, and the head is read
  from what the machine already knows. The `shared-versions` check, a quick check, asks no remote.

### `[check.shared_versions]`: the shared versions the manifests hold to

Read by the `shared-versions` quick check (`docs/checks.md`, 「The quick checks」), which a declaration
enables only with this table's `repository`.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `repository` | string | required | The repository whose root holds `shared-versions.toml`, by its declared name, found in the machine's registry of checkouts; the file is read as that repository's main branch holds it |
| `exclude` | list of globs | `[]` | The Cargo manifests, locks and package manifests the check leaves unread, such as the upstream folders a fork carries unchanged |

### `[check.shared_versions.exceptions.<crate>]`: a crate this repository takes otherwise

One table for each listed crate the repository genuinely needs at another version or feature set, the
variation standing in the repository that needs it. An exception naming neither a line nor a version, or
with an empty reason, is refused.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `reason` | string | required | Why the repository needs the crate otherwise, in English |
| `line` | string | none | The line the manifests write for the crate instead of the shared one, whole, its key included |
| `locked` | list of strings | `[]` | The versions of the crate the lock holds instead of the snapshot's |

### `[[check.step]]`: one static step each, in order

| Key | Type | Default | Meaning |
|---|---|---|---|
| `id` | string | required | The step's English id, which the output, the progress record and the times name |
| `command` | command | required | What the step runs |
| `dir` | path | the root | Where it runs |
| `lane` | string | none | `wide` or `narrow` to run in the testbed's turn; none runs where it stands |
| `when` | list of globs | every change | The paths that reach this step; a change touching none of them leaves it out |
| `retry_out_of_memory` | boolean | `false` | Run again where the kernel killed it for want of memory (`docs/testbed.md`, 「Memory」) |

A declaration declares no step that type-checks, formats or lints its Cargo workspace's own crates,
such as `cargo check`, `cargo clippy` or `cargo fmt`, through `cargo` or the Cargo verb and with
whatever flags: the static check and the change check both format and lint them wherever a change
marks a crate, the lints checking every crate as they lint it. A declared step adds only what the
testbed does not do.

Where either check runs the Cargo workspace's own formatting and lints, a declared step whose command
only repeats them, such as `cargo fmt --all --check` or
`cargo clippy --workspace --all-targets -- -D warnings` at the root, is left out and counts as passed
(`docs/checks.md`, 「The static check」 and 「The change check」). Where a change marks no crate, it
runs as declared.

## `[build]`: building a release

| Key | Type | Default | Meaning |
|---|---|---|---|
| `binaries` | list of strings | every binary target of the Cargo workspace | The binaries placed into a release's `bin/` |
| `cargo_profile` | string | `release` | The profile the release build uses |

The Cargo release build covers every package in the workspace, not the root package alone.

### `[[build.step]]`: one build step each, in order

Run in the build tree at the commit being deployed, after the Cargo release build where the
repository has a Cargo workspace. Each has `id`, `command`, `dir` and `lane` as a check step does. A
step that fails stops the deployment with every service untouched.

## `[deploy]`: putting a commit live

| Key | Type | Default | Meaning |
|---|---|---|---|
| `root` | path | none | The directory holding the deploy tree's link, `deploy-releases/` and `deploy-expand`, whose target directory the prune takes three days after the last deployment (`docs/deploy.md`, 「The releases」); a repository with none deploys nothing |
| `link` | string | `deploy` | The deploy tree's link name inside `root` |
| `data_root` | path | required with `root` | The repository's runtime data, inside no checkout |
| `env_links` | list of tables | `[]` | Each with `link`, a path inside a release, and `target`, a file under the data root; every target must exist |
| `units` | list of globs | every tracked `*.service`, `*.timer` and `*.slice` | Where the commit's unit files stand |
| `health` | command | none | Answers 0 once the `bot` service is healthy; asked until it does, the unit gives up starting or `health_wait_seconds` pass (`docs/deploy.md`, 「What the deploy run does, in order」), each ask waited for until it ends or the bound passes |
| `health_wait_seconds` | integer | the testbed's policy, 600 | How long the wait for the `health` may last, after a restart and again after a rollback's; where it passes before the health answers, the deployment rolls back and fails |
| `stay_up_seconds` | integer | none | Read and given no effect: the `bot` service is up once its health answers and it is read running |
| `quiet` | command | none | Run from the release just before the record and the flip: answers 0 where no work stands under way that a restart would cut and 1 where some does, its output naming it, asked again until it answers 0, however long that takes; any other answer is a quiet that could not tell, which fails the deployment with nothing changed once it repeats the policy's number of asks running (`docs/deploy.md`, 「What the deploy run does, in order」) |
| `quiet_wait_seconds` | integer | none | Read and given no effect |
| `gate` | boolean | `true` | Run the static check and the suite before deploying |

### `[[deploy.step]]`: one declared step each, in order

| Key | Type | Default | Meaning |
|---|---|---|---|
| `id` | string | required | The step's English id |
| `command` | command | required | What it runs, from the release |
| `phase` | string | required | `before-flip`: after the build, before the record and the flip, a failure stops the deployment; `after-health`: once the `bot` service is up and the console probe passed, a failure fails the deployment, which rolls back as a service that did not come up does |
| `timeout_seconds` | integer | none | Read and given no effect: a step runs until it ends |

### `[deploy.console]`: the console screen the repository brings

The table's presence says the repository's service brings a screen to a console, a bundle under bmo-ui's
console plugin contract (bmo-ui's `docs/console-bundles.md`). Once the service is up, the deployment probes
its console face against the contract (`docs/deploy.md`, 「The console probe」); a repository bringing no
screen declares no such table.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `reach` | command | required | Run from the release once the service is up: answers on its standard output one JSON object naming how a console reaches the service, `address`, the base address of the service's API as a console calls it, `consoleToken`, the credential the console calls it with, and `token`, the service's own token, which the console's routes refuse. `address` and `consoleToken` are non-empty strings; `token` is one too where the service holds a token of its own, and is left out or answered `null` where it holds none, as a service whose callers sign in by session holds none. Where each stands is the repository's own |

### `[deploy.backup]`: the backup a deployment takes

| Key | Type | Default | Meaning |
|---|---|---|---|
| `dir` | path | `<root>/backups` | The backup directory |
| `name` | string | the repository's name | The backup file's name before its time |
| `snapshot` | command | none | Writes a consistent copy of what the services are writing into `BMO_BACKUP_SNAPSHOT_DIR`, with a manifest beside it |
| `verify` | command | none | Holds a snapshot taken back out of the pack, in `BMO_BACKUP_SNAPSHOT_DIR`, to its manifest; 0 when it matches |
| `exclude` | list of paths | `[]` | Paths under the data root left out of the pack |
| `remote` | string | none | The S3 address the `backups` verb sends to; none keeps backups local |
| `profile` | string | none | The AWS command's profile for the remote |
| `keep` | integer | 5 | How many backups stand remotely |

## `[runner]`: placement

| Key | Type | Default | Meaning |
|---|---|---|---|
| `placement` | URL | none | The placement endpoint (`docs/boundary.md`, 「Placement」); `BMO_TESTBED_PLACEMENT` overrides it, and none runs every run where it was started |
| `local_variables` | list of strings | `[]` | The variables this repository's runs read that name what only the machine they start on reaches, such as the address of a server on its loopback; a run whose environment sets one to a value that is not empty is never placed elsewhere |
| `local_files` | list of paths | `[]` | The files, kept out of git, that this repository's runs read and that name what only the machine they start on reaches, such as a test environment pointing at servers on its loopback; a run started in a checkout holding one, a link to a file counting as the file, is never placed elsewhere |

## `[events]`: where events go

| Key | Type | Default | Meaning |
|---|---|---|---|
| `hook` | command | none | Run once for each event, its name as the first argument and its payload on standard input (`docs/boundary.md`, 「Events」); `BMO_TESTBED_EVENT_HOOK` overrides it |
| `timeout_seconds` | integer | none | Read and given no effect: the hook is waited for until it ends |
