import { expect, test } from "bun:test";

// A file whose one test passes and reaches no stand-in.
test("passes", () => {
  expect(1 + 1).toBe(2);
});
