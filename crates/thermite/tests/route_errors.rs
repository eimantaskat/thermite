// trybuild compiles each file and compares the compiler's output with the .stderr file next to it
#[test]
fn route_attributes_report_mistakes_at_compile_time() {
    trybuild::TestCases::new().compile_fail("tests/route_errors/*.rs");
}
