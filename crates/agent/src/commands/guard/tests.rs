//! The parts of `agent guard-commit` that read text: the subject of a message, the parent of a process and whether a
//! command line amends.

use super::*;

// Rule: the subject is the first line that is not blank and not a comment, as git's own cleanup reads a message.
#[test]
fn the_subject_is_the_first_line_that_is_no_comment() {
    let cases: [(&str, Option<&str>); 7] = [
        ("TEST-FIX: the case\n\nbody\n", Some("TEST-FIX: the case")),
        ("Format the test\n", Some("Format the test")),
        ("\n\n  \nafter blanks\n", Some("after blanks")),
        (
            "# Please enter the commit message\nTEST-FIX: after a comment\n",
            Some("TEST-FIX: after a comment"),
        ),
        ("  indented subject  \nbody", Some("indented subject")),
        ("# only a comment\n\n", None),
        ("", None),
    ];

    for (message, want) in cases {
        assert_eq!(subject_of(message), want, "{message:?}");
    }
}

// Rule: the parent is the fourth field of the stat line, which comes after the name of the command, and that name may
// hold spaces and brackets.
#[test]
fn the_parent_is_read_after_the_name_of_the_command() {
    let cases: [(&str, Option<u32>); 5] = [
        ("4242 (git) S 4100 4242 4100 0 -1 4194560", Some(4100)),
        ("77 (my (odd) name) R 12 77 12 0 -1", Some(12)),
        ("1 (systemd) S 0 1 1 0 -1", Some(0)),
        ("12 (git) S", None),
        ("nonsense", None),
    ];

    for (stat, want) in cases {
        assert_eq!(parent_of(stat), want, "{stat:?}");
    }
}

// Rule: a command line amends when one of its arguments is exactly `--amend`; the arguments are separated by NUL.
#[test]
fn a_command_line_amends_when_one_argument_is_amend() {
    let cases: [(&[u8], bool); 6] = [
        (b"git\0commit\0--amend\0-m\0Format\0", true),
        (b"git\0commit\0-a\0--amend\0--no-edit\0", true),
        (b"git\0-c\0core.hooksPath=/x\0commit\0--amend", true),
        (b"git\0commit\0-m\0--amend-me\0", false),
        (b"git\0commit\0-m\0use --amend later\0", false),
        (b"", false),
    ];

    for (cmdline, want) in cases {
        assert_eq!(is_amend(cmdline), want, "{cmdline:?}");
    }
}
