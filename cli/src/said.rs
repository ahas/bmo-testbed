//! What a verb says: its lines to standard output, its lines to standard error, and the code it ends with,
//! gathered before anything is printed so a test reads them as the binary would print them.

/// A verb's answer.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Said {
  pub out: Vec<String>,
  pub err: Vec<String>,
  pub code: u8,
}

impl Said {
  /// The answer printed; the code it ends with.
  pub fn print(&self) -> u8 {
    for line in &self.out {
      println!("{line}");
    }

    for line in &self.err {
      eprintln!("{line}");
    }

    self.code
  }
}
