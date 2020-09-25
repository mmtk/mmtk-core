// Each module should only contain one #[test] function.
// We should run each module in a separate test process, as we do not have proper
// setup/teardown procedure for MMTk instances.
mod issue139;