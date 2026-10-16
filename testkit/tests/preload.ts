import { refused, standInModule } from "../test-stand-ins";
import { setUpTestHome } from "../test-home";

// The test kit's own tests set up their home as a consumer's preload would, from a neutral ask.

// The caller's environment the ask takes away, stood in here so the tests can see it gone.
process.env.TESTKIT_EMPTIED = "from the caller";
process.env.TESTKIT_REMOVED = "from the caller";
process.env.TESTKIT_GONE_ONE = "from the caller";
process.env.TESTKIT_GONE_TWO = "from the caller";
process.env.DBUS_SESSION_BUS_ADDRESS = "unix:path=/nowhere";
process.env.BMO_TESTBED_HELD = "a holder's mark";

export const STOOD_IN_MODULE = "testkit-stood-in-module";

standInModule(STOOD_IN_MODULE, {
  Client: refused(STOOD_IN_MODULE, "a client"),
  version: "0.0.0-test-stand-in",
});

setUpTestHome({
  emptied: ["TESTKIT_EMPTIED"],
  removed: ["TESTKIT_REMOVED"],
  removedPrefixes: ["TESTKIT_GONE_"],
  standIns: ["testkit-stand-in"],
  directories: { TESTKIT_PLACE: "place" },
});
