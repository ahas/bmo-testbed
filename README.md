**English** | [한국어](README.ko.md) | [日本語](README.ja.md) | [简体中文](README.zh-CN.md)

# bmo-testbed

A general-purpose testbed for one machine shared by many callers and many repositories: one queue for
every heavy test and build command, a whole-suite runner with change selection, quick static checks,
and a release-based deployment of a repository onto the machine it runs on.

- **What it knows is mechanism.** It knows no repository by name. A repository that uses it declares
  how it is tested, checked, built and deployed in one file at its root, `testbed.toml`
  (`docs/declaration.md`), and everything that differs between repositories stands there.
- **What it offers is one binary,** `bmo-testbed`, the crates it is built from, and one tool the tool
  host runs, `testbed/deploy`, through which a harness's session puts a commit live. The verbs, their
  exit codes, the environment they read, the declaration they read, the events they send, the HTTP
  service `serve` starts, the harness plugin `mcp` starts, the hosted deploy tool and the placement
  endpoint they ask are its whole boundary (`docs/boundary.md`), the routes and tools stated in
  `docs/openapi.json`.
- **It is an upstream:** a harness takes it as its first entry point through its MCP server and the
  `bmo-testbed` skill, with nothing else running.
- **It depends on no other repository** but the base libraries `bmo-chat`, `bmo-chat-client`,
  `bmo-console-face`, `bmo-text` and `bmo-ui`, and a consumer may take its test kit as a dev-dependency.

## Documents

| Document | What it states |
|---|---|
| `docs/testbed.md` | The queue: lanes, turns, the front and the next, waiting, width, the result line, languages, placement |
| `docs/suite.md` | The whole-suite run: sweep, lanes, fence, the test database, what failed runs once more, the daily whole run, the nightly round over the machine's repositories |
| `docs/checks.md` | Which tests a change runs, the static check, the quick checks, the change check, imports, the test count |
| `docs/deploy.md` | Releases, adoption, unit marks, the deploy run, progress, the hosted deploy tool, rollback, refusals, backups, the Cargo build directory |
| `docs/install.md` | The install: a preset of an index's presets file laid onto a fresh machine, each repository's `install.toml`, the steps in order, each printed as one JSON line |
| `docs/supervise.md` | The supervisor: the set's services run on a machine with no systemd user manager, from the same unit files, with stand-ins for `systemctl`, `systemd-run` and `journalctl` |
| `docs/backup.md` | The data backup: what is backed up, where, how often, the rotation, the restore drill, restoring by hand, the settings |
| `docs/declaration.md` | The `testbed.toml` schema |
| `docs/boundary.md` | Every verb, exit code, status field, environment variable, event, the record, the HTTP service, the harness plugin, the hosted deploy tool, the placement endpoint and the version rules |
| `docs/openapi.json` | The HTTP service's routes and members and the harness plugin's tools, generated from the code |
| `docs/PERSONAL-DATA.md` | What may never stand in this repository: a person's name, address, path, device or network details, an agent's or product's name, and the check that holds it |

## Layout

| Folder | What it holds |
|---|---|
| `core/` | The queue, the lanes, the turn, the run and its result, the languages, memory limits, the declaration reader and the policy loader |
| `suite/` | The whole-suite runner, change selection, the static check, the quick checks, the Cargo verb and the build directory prune |
| `deploy/` | Releases, the deploy tree link, unit links, the deploy run, progress, rollback and backups |
| `install/` | The install: the presets file, the install declaration, a template's marks, and the run's stages against the machine |
| `supervise/` | The supervisor: unit files read, their processes run, restarted and stopped, the control socket and the stand-ins |
| `backup/` | The data backup: a machine's databases and data folders dumped, packed, sent to its destinations, rotated and restored by a drill |
| `cli/` | The `bmo-testbed` binary: reading the words of each verb and handing them to the crates |
| `client/` | The published client: the HTTP service's members, queries and error codes, and the client calling each route |
| `record/` | The record every verb writes and the service reads: test runs, deployments, log lines, restarts and events |
| `service/` | The HTTP service over the record and the lanes, with its event stream |
| `plugin/` | The harness plugin: the MCP server's tools, the OpenAPI document and the `bmo-testbed` skill |
| `tools/` | The tools the tool host runs, each a crate beside its manifest and input schema: `deploy`, which puts a commit live for a harness's session |
| `test-support/` | The fixed ids a Rust test names its people, rooms and units by, the same as the test kit's |
| `scratch-database/` | Per-test PostgreSQL databases cloned from a template a consumer's migrator makes |
| `testkit/` | The TypeScript test kit a consumer preloads: the test home and the stand-ins |
| `units/` | The user units this repository's deployment keeps: the Cargo build directory's prune, the data backup's snapshot, media and drill, their timers, and the slices every unit the testbed starts or ships runs in |
| `policy/` | The testbed's own fixed numbers and words, read by `core`'s policy loader |
| `docs/` | The documents above |

At the root stand the Cargo workspace manifest, which holds the version, and its lockfile, the
formatter's settings, this repository's own `testbed.toml` and `install.toml`, and `CLAUDE.md`.

## Version

- **One version for the whole repository,** the workspace's, which is the BMO set's: it moves by semantic
  versioning only at a release of the whole set the owner declares, and a deployment between releases is
  named for the day's nightly (`docs/boundary.md`, 「Versions」; `docs/deploy.md`, 「The deployed version」).
- **A declaration names the major of its schema,** and a testbed reading another major refuses it, so a
  consumer moves to a new schema on purpose; the set's version moves it not at all.
