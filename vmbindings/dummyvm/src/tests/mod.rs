/// These tests are integration tests. 
/// However, we cannot put them to a 'tests' folder next to 'src'. This crate is a cdylib, and using 
/// the 'tests' folder for integration tests does not work.

/// We have to run each of these tests in a separate process, 
/// as we only have one MMTk instance and we do not have proper setup/teardown for the instance.

/// Run the tests as below:
/// * cargo +nightly test gcbench --features nogc
/// * cargo +nightly test fixed_live --features nogc

mod fixed_live;
mod gcbench;