//! The testbed's hosted deploy tool (docs/deploy.md, 「The hosted deploy tool」; docs/boundary.md, 「The
//! hosted deploy tool」): the tool host hands it one call of `testbed/deploy`, and it puts the commit live
//! through the deploy verb from outside the harness the session stands in, waits the deployment out and
//! answers what came of it.

mod call;
mod checkout;
mod deployment;
mod following;

use bmo_testbed_core::policy::deploy_tool::DEPLOY_TOOL;
use bmo_testbed_core::policy::fill_marks;
use serde_json::{Value, json};

pub use call::{Answer, Asked, Content, Handed, read_handed};
pub use checkout::{find_checkout, same_name};
pub use deployment::{
  Arguments, Begun, DATA_ROOT_VARIABLE, Ground, HARNESS_VARIABLE, PLACED, TESTBED, deploy, outcome,
  read_arguments, recorded, release_testbed, unit_of,
};
pub use following::{Course, newest, of_commit, of_repository};

/// What a request is answered with on standard output, or why it is answered with none: a kind the tool
/// does not serve, which the host takes as a failed call.
pub fn answer(handed: &Handed, ground: &Ground) -> Result<Value, String> {
  let said = &DEPLOY_TOOL.lines;

  match &handed.asked {
    Asked::Call { entry, arguments } => {
      let answered = if *entry != DEPLOY_TOOL.entry {
        Answer::error(fill_marks(&said.other_entry, &[("ours", &DEPLOY_TOOL.entry), ("asked", entry)]))
      } else {
        match read_arguments(arguments) {
          Ok(arguments) => {
            // Who marks a deployment first is the session's holder, as its record names it.
            let asked_by = Ground {
              asker: handed.session["holderName"].as_str().map(str::to_owned),
              ..ground.clone()
            };

            deploy(&arguments, &asked_by)
          }
          Err(why) => Answer::error(why),
        }
      };

      serde_json::to_value(answered).map_err(|why| why.to_string())
    }
    Asked::ResourcesList => Ok(json!({ "resources": [] })),
    Asked::ResourcesRead { .. } => Ok(json!({ "error": said.no_resources })),
    Asked::Other => Err(said.other_kind.clone()),
  }
}
