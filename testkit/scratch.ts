import { existsSync, mkdtempSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { afterAll } from "bun:test";

// A directory a test may write in, taken away again when the file that asked for it is done
// (docs/suite.md, 「The test home」). Only a test calls this: it registers its own cleanup with the
// test runner, which nothing outside a test run has.
//
// A directory made here arrives with its own removal, so leaving one behind is not something a test
// can do by forgetting. The removal is registered with afterAll, which the runner runs whatever
// became of the tests inside it: one that passed, one that failed an expectation and one that threw
// all leave the same nothing behind. What no cleanup inside a process survives is that process being
// killed outright, so a run also takes away what an earlier killed run left, by the prefix it was
// asked for and by age, never by a sweep of the whole directory.

// What every scratch directory's name begins with, so nothing else under the temporary directory can
// be taken for one of these.
export const SCRATCH_MARK = "bmo-";

// The file inside a scratch directory that says the test kit made it.
export const SCRATCH_MARKER = ".bmo-test-scratch";

// How old a leftover has to be before a run takes it away. A whole suite is seconds, so anything this
// old belongs to a run that is over, and two runs at once never reach into each other's work.
const staleMs = 60 * 60_000;

const swept = new Set<string>();
let entries: string[] | null = null;

export function scratchDir(prefix: string): string {
  if (!prefix.startsWith(SCRATCH_MARK)) {
    throw new Error(`a scratch directory is named for the test kit: ${prefix} does not begin with ${SCRATCH_MARK}`);
  }

  sweepStale(prefix);

  const dir = mkdtempSync(join(tmpdir(), prefix));

  writeFileSync(join(dir, SCRATCH_MARKER), "owned by scratchDir\n");

  afterAll(() => {
    rmSync(dir, { recursive: true, force: true });
  });

  return dir;
}

// What a killed run left under one prefix. Directories holding the marker alone: a program may write
// real files of its own under names of this shape, and a file is never something this made, so a file
// is never something this takes away.
function sweepStale(prefix: string): void {
  if (swept.has(prefix)) {
    return;
  }

  swept.add(prefix);

  try {
    entries ??= readdirSync(tmpdir());
  } catch {
    return;
  }

  const cutoff = Date.now() - staleMs;

  for (const name of entries) {
    if (!name.startsWith(prefix)) {
      continue;
    }

    const path = join(tmpdir(), name);

    try {
      const held = statSync(path);

      if (held.isDirectory() && held.mtimeMs < cutoff && existsSync(join(path, SCRATCH_MARKER))) {
        rmSync(path, { recursive: true, force: true });
      }
    } catch {
      // Gone already, or not this user's to read: either way there is nothing to take away.
    }
  }
}
