# bmo-testbed

A general-purpose testbed for one machine shared by many callers and many repositories: one queue for
every heavy test and build command, a whole-suite runner with change selection, quick static checks,
and a release-based deployment of a repository onto the machine it runs on.

- **What it knows is mechanism.** It knows no repository by name. A repository that uses it declares
  how it is tested, checked, built and deployed in one file at its root, `testbed.toml`
  (`docs/declaration.md`), and everything that differs between repositories stands there.
- **What it offers is one binary,** `bmo-testbed`, and the crates it is built from. The verbs, their
  exit codes, the environment they read, the declaration they read, the events they send, the HTTP
  service `serve` starts, the harness plugin `mcp` starts and the placement endpoint they ask are its
  whole boundary (`docs/boundary.md`), the routes and tools stated in `docs/openapi.json`.
- **It is an upstream:** a harness takes it as its first entry point through its MCP server and the
  `ahas-testbed` skill, with nothing else running.
- **It depends on no other repository** but the base libraries `bmo-chat`, `bmo-chat-client`,
  `bmo-text` and `bmo-ui`, and a consumer may take its test kit as a dev-dependency.

## Documents

| Document | What it states |
|---|---|
| `docs/testbed.md` | The queue: lanes, turns, the front and the next, waiting, width, the result line, languages, placement |
| `docs/suite.md` | The whole-suite run: sweep, lanes, guards, fence, the test database, what failed runs once more, the daily whole run |
| `docs/checks.md` | Which tests a change runs, the static check, the quick checks, the change check, imports, the test count |
| `docs/deploy.md` | Releases, adoption, unit marks, the deploy run, progress, rollback, refusals, backups, the Cargo build directory |
| `docs/declaration.md` | The `testbed.toml` schema |
| `docs/boundary.md` | Every verb, exit code, status field, environment variable, event, the record, the HTTP service, the harness plugin, the placement endpoint and the version rules |
| `docs/openapi.json` | The HTTP service's routes and members and the harness plugin's tools, generated from the code |

## Layout

| Folder | What it holds |
|---|---|
| `core/` | The queue, the lanes, the turn, the run and its result, the languages, memory limits, the declaration reader and the policy loader |
| `suite/` | The whole-suite runner, change selection, the static check, the quick checks, the Cargo verb and the build directory prune |
| `deploy/` | Releases, the deploy tree link, unit links, the deploy run, progress, rollback and backups |
| `cli/` | The `bmo-testbed` binary: reading the words of each verb and handing them to the crates |
| `client/` | The published client: the HTTP service's members, queries and error codes, and the client calling each route |
| `record/` | The record every verb writes and the service reads: test runs, deployments, log lines, restarts and events |
| `service/` | The HTTP service over the record and the lanes, with its event stream |
| `plugin/` | The harness plugin: the MCP server's tools, the OpenAPI document and the `ahas-testbed` skill |
| `test-support/` | The fixed ids a Rust test names its people, rooms and units by, the same as the test kit's |
| `scratch-database/` | Per-test PostgreSQL databases cloned from a template a consumer's migrator makes |
| `testkit/` | The TypeScript test kit a consumer preloads: the test home and the stand-ins |
| `units/` | The user units this repository's deployment keeps: the Cargo build directory's prune and its timer |
| `policy/` | The testbed's own fixed numbers and words, read by `core`'s policy loader |
| `docs/` | The documents above |
| `graph/` | The code dependency graph: one JSON file per area, a node per feature with its files and the features it needs, written by the graph tool and never by hand |

At the root stand the Cargo workspace manifest, which holds the version, and its lockfile, the
formatter's settings, this repository's own `testbed.toml`, and `CLAUDE.md`.

## Version

- **One version for the whole repository,** the workspace's, over the boundary that `docs/boundary.md`
  states: `0.1.0` and a `-rev.N` revision until the first release, and semantic versioning from it
  (`docs/boundary.md`, 「Versions」).
- **A declaration names the major it is written for,** and a testbed of another major refuses it, so
  a consumer moves to a new major on purpose.
