//! The supervise crate's tests, one module to an area, built as one test binary. Every test stands in
//! temporary directories of its own, its units stand-ins written for it.

mod env_file;
mod manager;
mod places;
mod stand_in;
mod support;
mod unit;
