//! `supervise` (docs/supervise.md): the supervisor served in the foreground, its units' states read, the lines
//! that put a command under it, and its stop.

use std::path::PathBuf;

use bmo_testbed_core::policy::supervise::SUPERVISE;
use bmo_testbed_core::verb::{REFUSED_EXIT_CODE, data_root};
use bmo_testbed_supervise::control::{Request, ask_blocking};
use bmo_testbed_supervise::manager::with_stand_ins;
use bmo_testbed_supervise::places::Places;
use bmo_testbed_supervise::serve::{Refused, serve};
use bmo_testbed_supervise::unit::Specifiers;

use crate::words::SuperviseWord;

fn places(given: Option<PathBuf>) -> Places {
  Places::of_this_process(&given.unwrap_or_else(data_root))
}

pub fn supervise(word: SuperviseWord) -> u8 {
  match word {
    SuperviseWord::Serve { data_root, anyway } => {
      let places = places(data_root);
      let binary = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("bmo-testbed"));
      let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(why) => {
          eprintln!("{why}");
          return 1;
        }
      };
      let served =
        runtime.block_on(serve(places, Specifiers::of_this_process(), &binary, anyway, None, &|line| {
          eprintln!("{line}");
        }));

      match served {
        Ok(()) => 0,
        Err(refused @ (Refused::SystemdStands | Refused::Held(_))) => {
          eprintln!("{}", refused.said());
          REFUSED_EXIT_CODE
        }
        Err(refused) => {
          eprintln!("{}", refused.said());
          1
        }
      }
    }
    SuperviseWord::Status { json, data_root } => {
      let answer = match ask_blocking(&places(data_root).socket(), &Request::Show { units: Vec::new() }) {
        Ok(answer) => answer,
        Err(why) => {
          eprintln!("{why}");
          return 1;
        }
      };

      if json {
        println!("{}", serde_json::json!({ "units": answer.states }));
      } else {
        for state in &answer.states {
          println!(
            "{} {}/{} pid {} restarts {} health {}",
            state.id,
            state.active_state,
            state.sub_state,
            state.main_pid,
            state.n_restarts,
            serde_json::to_value(state.health)
              .ok()
              .and_then(|value| value.as_str().map(str::to_owned))
              .unwrap_or_default()
          );
        }
      }

      0
    }
    SuperviseWord::Env { data_root } => {
      let places = places(data_root);
      let path = std::env::var("PATH").unwrap_or_default();

      println!("PATH={}", with_stand_ins(&places.stand_ins(), &path));
      println!("{}={}", SUPERVISE.places.variable, places.socket().display());
      0
    }
    SuperviseWord::StopAll { data_root } => {
      eprintln!("{}", SUPERVISE.said.stopping);

      match ask_blocking(&places(data_root).socket(), &Request::Shutdown) {
        Ok(answer) => {
          eprintln!("{}", answer.out);
          0
        }
        Err(why) => {
          eprintln!("{why}");
          1
        }
      }
    }
  }
}
