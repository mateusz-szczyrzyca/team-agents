//! The built-in table of markers, per file extension.

use super::*;

/// Lines of files with one extension and the markers each has.
fn expect(extension: &str, cases: &[(&str, &[Marker])]) {
    for (line, markers) in cases {
        assert_eq!(markers_of(extension, line), *markers, "{extension}: {line}");
    }
}

// Rule: a Go test is skipped by a call of its own at the start of a line; a lint is suppressed by a `nolint` or `nosec`
// comment, which may follow code; `testing.Testing()` and the `test.` flags tell whether the code runs under test.
#[test]
fn go_lines_are_read_for_skips_suppressions_and_a_look_at_the_test_run() {
    expect(
        "go",
        &[
            ("\tt.Skip(\"not now\")", &[Marker::SkipsTest]),
            ("t.Skipf(\"%s\", why)", &[Marker::SkipsTest]),
            ("    s.T().SkipNow()", &[Marker::SkipsTest]),
            ("t.SkipNow()", &[Marker::SkipsTest]),
            ("// t.Skip(\"commented out\")", &[]),
            ("assert.Equal(t, \"t.Skip(\", got)", &[]),
            ("x := 1 //nolint:errcheck", &[Marker::SuppressesLint]),
            ("// nolint", &[Marker::SuppressesLint]),
            ("cmd := exec.Command(x) // #nosec G204", &[Marker::SuppressesLint]),
            ("if testing.Testing() {", &[Marker::ChecksUnderTest]),
            ("f := flag.Lookup(\"test.v\")", &[Marker::ChecksUnderTest]),
            ("x := 1", &[]),
        ],
    );
}

// Rule: Rust skips a test with `#[ignore]`, suppresses a lint with `#[allow(…)]` and asks about the test run with
// `cfg!(test)`; an attribute starts its line.
#[test]
fn rust_lines_are_read_for_ignore_allow_and_cfg_test() {
    expect(
        "rs",
        &[
            ("#[ignore]", &[Marker::SkipsTest]),
            ("    #[ignore = \"slow\"]", &[Marker::SkipsTest]),
            ("    #[allow(dead_code)]", &[Marker::SuppressesLint]),
            ("#![allow(unused)]", &[Marker::SuppressesLint]),
            ("if cfg!(test) {", &[Marker::ChecksUnderTest]),
            ("#[cfg(test)]", &[]),
            ("let attribute = \"#[ignore]\";", &[]),
            ("// #[allow(dead_code)]", &[]),
        ],
    );
}

// Rule: `#[expect(…)]` suppresses a lint as `#[allow(…)]` does — it is the spelling the rules allow, and still has to be
// named — at the start of its line, for an item or a whole module.
#[test]
fn rust_expect_attributes_suppress_a_lint() {
    expect(
        "rs",
        &[
            (
                "#[expect(dead_code, reason = \"wired later\")]",
                &[Marker::SuppressesLint],
            ),
            ("    #[expect(clippy::too_many_lines)]", &[Marker::SuppressesLint]),
            ("#![expect(unused, reason = \"a stub\")]", &[Marker::SuppressesLint]),
            ("// #[expect(dead_code)]", &[]),
            ("let text = \"#[expect(x)]\";", &[]),
            ("value.expect(\"a reason\")", &[]),
        ],
    );
}

// Rule: an attribute that `cfg_attr` applies is read as if it stood alone: `#[cfg_attr(not(test), expect(…))]`, the
// form PR-5 gives a suppression that only some builds need, suppresses a lint, and `#[cfg_attr(miri, ignore)]` skips a
// test. A `cfg_attr` that applies anything else is no marker.
#[test]
fn rust_attributes_that_cfg_attr_applies_are_read_like_attributes() {
    expect(
        "rs",
        &[
            (
                "#[cfg_attr(not(test), expect(dead_code, reason = \"wired in part 3/4\"))]",
                &[Marker::SuppressesLint],
            ),
            (
                "    #[cfg_attr(feature = \"x\", allow(unused))]",
                &[Marker::SuppressesLint],
            ),
            ("#![cfg_attr(not(test), expect(dead_code))]", &[Marker::SuppressesLint]),
            (
                "#[cfg_attr(not(test), expect(dead_code, reason = \"für später\"))]",
                &[Marker::SuppressesLint],
            ),
            ("#[cfg_attr(miri, ignore)]", &[Marker::SkipsTest]),
            ("#[cfg_attr(windows, ignore = \"no fork\")]", &[Marker::SkipsTest]),
            ("#[cfg_attr(feature = \"serde\", derive(Serialize))]", &[]),
            ("#[cfg_attr(test, derive(Debug))]", &[]),
            ("#[cfg_attr(docsrs, doc(cfg(feature = \"ignored\")))]", &[]),
            ("#[cfg_attr(test, my_allow(x))]", &[]),
            ("// #[cfg_attr(miri, ignore)]", &[]),
            ("let text = \"#[cfg_attr(miri, ignore)]\";", &[]),
        ],
    );
}

// Rule: what a string of a `cfg_attr` says is no attribute it applies; what follows a string still is.
#[test]
fn rust_words_inside_a_string_of_a_cfg_attr_are_no_markers() {
    expect(
        "rs",
        &[
            (
                "#[cfg_attr(docsrs, doc = \"ignore this, allow(x) and expect(y)\")]",
                &[],
            ),
            ("#[cfg_attr(feature = \"x\", allow(unused))]", &[Marker::SuppressesLint]),
            ("#[cfg_attr(feature = \"\\\"q\", ignore)]", &[Marker::SkipsTest]),
        ],
    );
}

// Rule: a bats test is skipped by `skip` as a word at the start of a line; shell code suppresses a lint with a
// `shellcheck disable=` comment and asks about the test run through a `BATS_` variable.
#[test]
fn shell_and_bats_lines_are_read_for_skip_shellcheck_and_bats_variables() {
    for extension in ["bats", "sh", "bash"] {
        expect(
            extension,
            &[
                ("skip \"not on this machine\"", &[Marker::SkipsTest]),
                ("  skip", &[Marker::SkipsTest]),
                ("skipped=1", &[]),
                ("echo skip", &[]),
                ("# shellcheck disable=SC2086", &[Marker::SuppressesLint]),
                ("#shellcheck   disable=SC1090", &[Marker::SuppressesLint]),
                ("source \"$f\" # shellcheck disable=SC1090", &[Marker::SuppressesLint]),
                ("[[ -n ${BATS_TEST_NAME:-} ]]", &[Marker::ChecksUnderTest]),
                ("# shellcheck shell=bash", &[]),
            ],
        );
    }
}

#[test]
fn python_lines_are_read_for_skips_noqa_type_ignore_nosec_and_pytest_variables() {
    expect(
        "py",
        &[
            ("    pytest.skip(\"x\")", &[Marker::SkipsTest]),
            ("@pytest.mark.skip(reason=\"x\")", &[Marker::SkipsTest]),
            ("@unittest.skipIf(sys.platform == 'win32', 'x')", &[Marker::SkipsTest]),
            ("self.skipTest(\"x\")", &[Marker::SkipsTest]),
            ("import os  # noqa: F401", &[Marker::SuppressesLint]),
            ("x = y  # type: ignore", &[Marker::SuppressesLint]),
            ("subprocess.run(args)  # nosec", &[Marker::SuppressesLint]),
            ("if \"PYTEST_CURRENT_TEST\" in os.environ:", &[Marker::ChecksUnderTest]),
            ("\"pytest\" in sys.modules", &[Marker::ChecksUnderTest]),
            ("x = 1", &[]),
        ],
    );
}

#[test]
fn script_lines_are_read_for_skips_ts_comments_eslint_and_test_environments() {
    for extension in ["js", "jsx", "ts", "tsx", "mjs", "cjs"] {
        expect(
            extension,
            &[
                ("it.skip('works', () => {})", &[Marker::SkipsTest]),
                ("  describe.skip('suite', () => {})", &[Marker::SkipsTest]),
                ("test.skip('x', fn)", &[Marker::SkipsTest]),
                ("xit('x', fn)", &[Marker::SkipsTest]),
                ("xdescribe('x', fn)", &[Marker::SkipsTest]),
                ("// @ts-ignore", &[Marker::SuppressesLint]),
                ("// @ts-expect-error", &[Marker::SuppressesLint]),
                ("// @ts-nocheck", &[Marker::SuppressesLint]),
                ("/* eslint-disable no-console */", &[Marker::SuppressesLint]),
                ("if (process.env.NODE_ENV === 'test') {", &[Marker::ChecksUnderTest]),
                ("const jest = process.env.JEST_WORKER_ID;", &[Marker::ChecksUnderTest]),
                ("it('works', () => {})", &[]),
            ],
        );
    }
}

// Rule: a line with several kinds of marker reports each once, in the order of the kinds.
#[test]
fn a_line_with_two_kinds_of_marker_reports_both_in_order() {
    assert_eq!(
        markers_of("go", "if testing.Testing() { t.Skip(\"x\") } //nolint"),
        [Marker::SkipsTest, Marker::SuppressesLint, Marker::ChecksUnderTest]
    );
}

// Rule: only the extensions of the table are scanned; a document or a file of another language has no markers.
#[test]
fn only_the_extensions_of_the_table_are_scanned() {
    for extension in [
        "go", "rs", "sh", "bash", "bats", "py", "js", "jsx", "ts", "tsx", "mjs", "cjs",
    ] {
        assert!(scans(extension), "{extension}");
    }
    for extension in ["md", "txt", "toml", "json", "yml", "", "GO"] {
        assert!(!scans(extension), "{extension:?}");
        assert_eq!(markers_of(extension, "#[ignore] //nolint skip"), [], "{extension:?}");
    }
}
