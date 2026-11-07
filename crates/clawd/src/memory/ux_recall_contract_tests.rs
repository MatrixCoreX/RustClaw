use super::boundary_test_fixtures as setup;
use super::*;

#[path = "ux_recall_contract_fixtures.rs"]
mod support;
use support::*;

#[path = "ux_recall_correction_tests.rs"]
mod corrections;
#[path = "ux_recall_disable_tests.rs"]
mod disabling;
