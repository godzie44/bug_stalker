use crate::common::DebugeeRunInfo;
use crate::common::TestHooks;
use crate::debugger_env;
use crate::{assert_no_proc, HW_APP};
use serial_test::serial;

#[test]
#[serial]
fn test_step_into() {
    debugger_env!(HW_APP, child, {
        let info = DebugeeRunInfo::default();
        let mut debugger = Debugger::new(HW_APP, child, TestHooks::new(info.clone())).unwrap();
        debugger.set_breakpoint_at_fn("main").unwrap();

        debugger.run_debugee().unwrap();
        assert_eq!(info.line.take(), Some(5));

        debugger.step_into().unwrap();
        assert_eq!(info.line.take(), Some(14));

        debugger.step_into().unwrap();
        assert_eq!(info.line.take(), Some(15));

        debugger.continue_debugee().unwrap();
        assert_no_proc!(child);
    });
}

#[test]
#[serial]
fn test_step_out() {
    debugger_env!(HW_APP, child, {
        let info = DebugeeRunInfo::default();
        let mut debugger = Debugger::new(HW_APP, child, TestHooks::new(info.clone())).unwrap();
        debugger.set_breakpoint_at_fn("main").unwrap();

        debugger.run_debugee().unwrap();
        assert_eq!(info.line.take(), Some(5));

        debugger.step_into().unwrap();
        assert_eq!(info.line.take(), Some(14));

        debugger.step_out().unwrap();
        assert_eq!(info.line.take(), Some(7));

        debugger.continue_debugee().unwrap();
        assert_no_proc!(child);
    });
}

#[test]
#[serial]
fn test_step_over() {
    debugger_env!(HW_APP, child, {
        let info = DebugeeRunInfo::default();
        let mut debugger = Debugger::new(HW_APP, child, TestHooks::new(info.clone())).unwrap();
        debugger.set_breakpoint_at_fn("main").unwrap();

        debugger.run_debugee().unwrap();
        assert_eq!(info.line.take(), Some(5));

        debugger.step_over().unwrap();
        assert_eq!(info.line.take(), Some(7));
        debugger.step_over().unwrap();
        assert_eq!(info.line.take(), Some(9));
        debugger.step_over().unwrap();
        assert_eq!(info.line.take(), Some(10));

        debugger.continue_debugee().unwrap();
        assert_no_proc!(child);
    });
}

#[test]
#[serial]
fn test_step_over_on_fn_decl() {
    debugger_env!(HW_APP, child, {
        let info = DebugeeRunInfo::default();
        let mut debugger = Debugger::new(HW_APP, child, TestHooks::new(info.clone())).unwrap();
        debugger
            .set_breakpoint_at_line("hello_world.rs", 14)
            .unwrap();

        debugger.run_debugee().unwrap();
        assert_eq!(info.line.take(), Some(14));

        debugger.step_over().unwrap();
        assert_eq!(info.line.take(), Some(15));

        debugger.continue_debugee().unwrap();
        debugger.continue_debugee().unwrap();
        assert_no_proc!(child);
    });
}
