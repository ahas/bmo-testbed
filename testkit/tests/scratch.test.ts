import { existsSync } from "node:fs";
import { basename, join } from "node:path";
import { tmpdir } from "node:os";

import { expect, test } from "bun:test";

import { SCRATCH_MARK, SCRATCH_MARKER, scratchDir } from "../scratch";

// A scratch directory (docs/suite.md, 「The test home」): named under the test kit's mark, made under
// the temporary directory, and carrying the marker its sweep goes by.

test("a scratch directory not named under the mark is refused", () => {
  expect(() => scratchDir("elsewhere-")).toThrow(/does not begin with bmo-/);
});

test("a scratch directory stands under the temporary directory with its marker", () => {
  const dir = scratchDir(`${SCRATCH_MARK}testkit-`);

  expect(basename(dir).startsWith(`${SCRATCH_MARK}testkit-`)).toBe(true);
  expect(dir.startsWith(tmpdir())).toBe(true);
  expect(existsSync(join(dir, SCRATCH_MARKER))).toBe(true);
});
