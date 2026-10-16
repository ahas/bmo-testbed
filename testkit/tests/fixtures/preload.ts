import { setUpTestHome } from "../../test-home";

// The home a fixture file runs in when a test of the test home starts it in a run of its own.
setUpTestHome({ standIns: ["testkit-stand-in"] });
