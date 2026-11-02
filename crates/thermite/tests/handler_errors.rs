// trybuild compiles each file and compares the compiler's output with the .stderr file next to it
#[test]
fn handler_mistakes_are_reported_at_compile_time() {
    trybuild::TestCases::new().compile_fail("tests/handler_errors/*.rs");
}
