//! A step's line (docs/install.md, 「What it prints」).

use bmo_testbed_install::{Line, Status};

#[test]
fn a_step_is_one_json_object_of_four_members() {
  let line = Line::new("token:web.server", Status::NeedsInput, "made", "bmo install --set web.server");
  let read: serde_json::Value = serde_json::from_str(&line.json()).unwrap();

  assert!(!line.json().contains('\n'));
  assert_eq!(read["step"], "token:web.server");
  assert_eq!(read["status"], "needs-input");
  assert_eq!(read["detail"], "made");
  assert_eq!(read["next"], "bmo install --set web.server");
  assert_eq!(read.as_object().unwrap().len(), 4);
}

#[test]
fn every_status_reads_as_its_word() {
  for (status, word) in [
    (Status::Ok, "ok"),
    (Status::Planned, "planned"),
    (Status::NeedsInput, "needs-input"),
    (Status::Failed, "failed"),
  ] {
    let read: serde_json::Value = serde_json::from_str(&Line::new("s", status, "", "").json()).unwrap();

    assert_eq!(read["status"], word);
  }
}
