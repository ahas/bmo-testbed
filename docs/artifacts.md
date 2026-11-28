# Unit artifacts: `pack` and a deployment from an artifact

## What it is

- **An artifact is one repository's release, built once and carried to another machine whole.** It is the
  release tree a deployment prepares (`docs/deploy.md`, 「The releases」) with only what the release runs on,
  packed into one file with a manifest and a signature. A machine that installs it compiles nothing,
  installs no package of a language's own and reads no checkout: every binary, every built console, every
  placed tool and every prompt the repository's deploy steps read stands in it already.
- **It is how a person's machine gets BMO.** The release builder packs each unit of the set at a release
  tag or at the head of main; the person's machine fetches the artifacts from the package host and deploys
  each from its file (「A deployment from an artifact」). The source road, a checkout deployed by its
  commit, stays the road of a machine that develops the set.
- **What a repository's release needs is that repository's own to declare** (「The declaration」); the testbed
  knows no unit by name.

## The words

```
bmo-testbed pack [--commit <sha>] [--out <dir>] [--key <file>]
bmo-testbed deploy --artifact <file> [--message <why>] [--dry-run]
```

- **`pack`** builds the repository at the commit, `HEAD` where none is named, into a release as a
  deployment would, with the release builder's floor (「The floor」), keeps what the declaration says the
  release runs on, and writes `<name>-<version>-<arch>.tar.zst` and its manifest `<name>-<version>-<arch>.json`
  under `--out`, the current directory where none is named. `--key` names the signing key's file, read as
  the policy's signer reads it; without one the manifest carries no signature, which a deployment from an
  artifact refuses unless the machine's settings allow unsigned artifacts.
- **`deploy --artifact`** puts the release an artifact holds live, as 「A deployment from an artifact」 states.

## The declaration

A repository's `testbed.toml` names what its release runs on in an `[artifact]` table:

| Key | Type | Meaning |
|---|---|---|
| `keep` | list of globs | The paths of the release, from its root, an artifact keeps besides the ones every artifact keeps |
| `drop` | list of globs | Paths under a kept one that the artifact leaves out, such as a folder of packages a built console no longer reads |
| `runtimes` | list of strings | The runtimes the release runs on beside its own binaries, each one the policy names (`bun`), packed into the artifact's `runtime/` folder |

- **Every artifact keeps** `bin/`, `deploy/`, `tools/`, `policy/`, `lang/`, `migrations/`, `testbed.toml`,
  `install.toml` and the `LICENSE`, each where it stands.
- **A repository with no `[artifact]` table packs nothing:** `pack` refuses it, naming the table, so a unit
  is only offered as an artifact once its repository has said what its release runs on.
- **Everything a deploy step builds moves to a build step.** A deploy step runs on the person's machine, so a
  repository whose deploy steps compile, install a language's packages or read a checkout outside the
  release declares that work as a `[[build.step]]` writing into the release instead, and `pack` refuses a
  declaration whose deploy step names `cargo`, `bun install`, `npm` or `git` (「What it refuses」).

## The floor

- **An artifact runs on every machine at or above the policy's floor:** Linux on the architecture it names,
  with the GNU C library at the policy's version or above, `2.35` (Ubuntu 22.04). `pack` builds Rust for that
  floor with `cargo zigbuild --target <arch>-unknown-linux-gnu.<floor>`, and reads every ELF file it keeps
  before it writes the artifact: one that asks for a newer symbol version of the C library refuses the pack,
  naming the file and the version.
- **A runtime the release names is packed at the policy's version,** its binary fetched from its own release
  and checked against the checksum the policy holds, so every artifact of a set carries the same one.

## The manifest

One JSON object beside the artifact:

| Member | Meaning |
|---|---|
| `name` | The repository's name |
| `version` | The deployed version the commit names (`docs/deploy.md`, 「The deployed version」) |
| `commit` | The commit it was built from |
| `arch` | `x86_64` or `aarch64` |
| `glibc` | The highest symbol version of the C library any kept file asks for |
| `sha256` | The artifact file's digest |
| `signature` | The digest signed by the set's signing key, base64; absent where the pack was not signed |

## A deployment from an artifact

- **It is a deployment as the source road's is from the release onward** (`docs/deploy.md`, 「What the deploy
  run does, in order」): the data root and backup checks, the deployment line, the backup, then the release
  laid, the steps before the flip, the flip, the restart, the wait for the services to stay up and the steps
  after it, with the same record, events and rollback.
- **It leaves out what a release from source needs:** no fetch and no commit picked from a checkout, no gate,
  no build tree and no build; the release is the artifact unpacked under `deploy-releases/<commit's first
  twelve>`, its declaration read from the `testbed.toml` it carries.
- **It checks the artifact first:** the digest against the manifest, the signature against the set's public
  key the policy names, and the floor against this machine's C library, refusing any that does not hold,
  with nothing changed.
- **The runtimes it carries stand first on its steps' `PATH`,** so a step or a service that runs `bun` runs the
  one the artifact carries.

## What it refuses

- `pack` refuses a repository with no `[artifact]` table, a deploy step that builds (「The declaration」), a
  kept file above the floor, and a build that fails.
- `deploy --artifact` refuses a digest that does not match, a missing or wrong signature, a floor above this
  machine's, and an artifact of a repository other than the deploy root's.
