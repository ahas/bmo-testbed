# The declaration: `testbed.toml`

- **One file at a repository's root,** read by every verb run inside that repository's checkout. It is
  where everything that differs between repositories stands: how the repository is tested, checked,
  built and deployed, and where its events go.
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
  without the input it reads, a deploy root without its data root, a test database naming only one of
  its migrations and its migrate command, and two steps of one table under the same id. Whether a path
  exists is asked by the verb that reads it, not here.

## The top level

| Key | Type | Default | Meaning |
|---|---|---|---|
| `testbed` | integer | required | The major version of the testbed this declaration is written for. A testbed of another major refuses it, naming both |
| `name` | string | the root directory's name | The repository's name, used in unit names, backup names and events |

## `[test]`: how the suite runs

| Key | Type | Default | Meaning |
|---|---|---|---|
| `languages` | list of strings | `[]` | The kinds of unit the suite gathers: `bun` (test files run by `bun test`), `node` (test files run by `node --test`), `cargo` (crates of the Cargo workspace), `browser` (test files in the browser lane) |
| `files` | list of globs | each declared language's own naming | The test files of the file languages, each run from the folder of its runner's configuration (`docs/suite.md`, 「Which files run」) |
| `browser` | list of globs | `[]` | Test files run one at a time in the browser lane |
| `guard_seconds` | integer | 240 | The whole-run guard |
| `file_guard_seconds` | integer | 90 | The per-file guard |
| `test_timeout_seconds` | integer | 30 | The per-test timeout handed to the runner |
| `memory_gb` | integer | 8 | The fence's memory ceiling, which `BMO_SUITE_MEMORY_GB` overrides |
| `setup` | list of commands | `[]` | Run in order to set up a fresh tree: a release, the daily whole run's tree, a bench copy on another machine |

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
| `discover` | boolean | `true` | Find every directory carrying its own package manifest and lockfile that the root workspace does not list |
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
| `quick` | list of strings | `[]` | The quick checks run: any of `wording`, `dashes`, `korean`, `layout`, `dependencies` |
| `wording_document` | path | none | The document holding the banned terms and their allowed uses; required by `wording` |
| `wording_files` | list of globs | every file | The files `wording` holds to the document |
| `people_text` | list of globs | `[]` | The files whose literals are text for a person; read by `dashes` |
| `korean_ledger` | path | none | The ledger of Korean lines allowed per code file; required by `korean` |
| `base_libraries` | list of strings | `[]` | The dependencies from other repositories the `dependencies` check allows beside registry ones |
| `main` | string | `main` | The main branch, the default base of a change |

### `[[check.step]]`: one static step each, in order

| Key | Type | Default | Meaning |
|---|---|---|---|
| `id` | string | required | The step's English id, which the output, the progress record and the times name |
| `command` | command | required | What the step runs |
| `dir` | path | the root | Where it runs |
| `lane` | string | none | `wide` or `narrow` to run in the testbed's turn; none runs where it stands |
| `when` | list of globs | every change | The paths that reach this step; a change touching none of them leaves it out |
| `retry_out_of_memory` | boolean | `false` | Run again where the kernel killed it for want of memory (`docs/testbed.md`, 「Memory」) |

## `[build]`: building a release

| Key | Type | Default | Meaning |
|---|---|---|---|
| `binaries` | list of strings | every binary target of the Cargo workspace | The binaries placed into a release's `bin/` |
| `cargo_profile` | string | `release` | The profile the release build uses |

### `[[build.step]]`: one build step each, in order

Run in the build tree at the commit being deployed, after the Cargo release build where the
repository has a Cargo workspace. Each has `id`, `command`, `dir` and `lane` as a check step does. A
step that fails stops the deployment with every service untouched.

## `[deploy]`: putting a commit live

| Key | Type | Default | Meaning |
|---|---|---|---|
| `root` | path | none | The directory holding the deploy tree's link, `deploy-releases/` and `deploy-expand`; a repository with none deploys nothing |
| `link` | string | `deploy` | The deploy tree's link name inside `root` |
| `data_root` | path | required with `root` | The repository's runtime data, inside no checkout |
| `env_links` | list of tables | `[]` | Each with `link`, a path inside a release, and `target`, a file under the data root; every target must exist |
| `units` | list of globs | every tracked `*.service` and `*.timer` | Where the commit's unit files stand |
| `health` | command | none | Answers 0 once the `bot` service is healthy; asked until it does, at most `health_wait_seconds` |
| `health_wait_seconds` | integer | 300 | The longest wait for the health |
| `stay_up_seconds` | integer | 30 | How long the `bot` service is watched staying up |
| `gate` | boolean | `true` | Run the static check and the suite before deploying |

### `[[deploy.step]]`: one declared step each, in order

| Key | Type | Default | Meaning |
|---|---|---|---|
| `id` | string | required | The step's English id |
| `command` | command | required | What it runs, from the release |
| `phase` | string | required | `before-flip`: after the build, before the record and the flip, a failure stops the deployment; `after-health`: once the `bot` service stays up, a failure is a warning |
| `timeout_seconds` | integer | 1200 | Past it the step is stopped, failing as its phase says |

### `[deploy.backup]`: the backup a deployment takes

| Key | Type | Default | Meaning |
|---|---|---|---|
| `dir` | path | required | The backup directory |
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

## `[events]`: where events go

| Key | Type | Default | Meaning |
|---|---|---|---|
| `hook` | command | none | Run once for each event, its name as the first argument and its payload on standard input (`docs/boundary.md`, 「Events」); `BMO_TESTBED_EVENT_HOOK` overrides it |
| `timeout_seconds` | integer | 30 | Past it the hook is stopped |
