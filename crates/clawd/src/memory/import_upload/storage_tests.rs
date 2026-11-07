use super::test_fixtures::*;
use super::*;

#[path = "accounting_tests.rs"]
mod accounting;
use std::io::Read;

#[path = "chunks_tests.rs"]
mod chunks;
#[path = "identity_tests.rs"]
mod identity;
#[path = "leases_tests.rs"]
mod leases;
#[path = "reader_tests.rs"]
mod reader;
#[path = "schema_tests.rs"]
mod schema;
