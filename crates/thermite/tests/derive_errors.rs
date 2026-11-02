// trybuild compiles each file and compares the compiler's output with the .stderr file next to it
#[test]
fn derive_model_reports_mistakes_at_compile_time() {
    trybuild::TestCases::new().compile_fail("tests/derive_errors/*.rs");
}
