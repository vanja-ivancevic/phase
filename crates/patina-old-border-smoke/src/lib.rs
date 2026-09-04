//! Patina's compact old-border verification boundary.
//!
//! This package deliberately depends on `phase-engine` as a normal library.
//! Its tests exercise public parser and runtime APIs without compiling Phase's
//! own `#[cfg(test)]` corpus or the monolithic integration-test binary. Add
//! one deterministic, reusable-mechanism scenario per Patina coverage batch;
//! retain Phase's full suite for milestone gates and upstream integration.
