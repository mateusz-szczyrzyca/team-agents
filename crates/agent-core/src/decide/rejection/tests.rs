//! The reading of a lead's `RETURNED:` lines: a table of what is a return and what is not.

use super::*;

/// The items of `message` as text, in order.
fn returned(message: &str) -> Vec<String> {
    returned_items(message).iter().map(ToString::to_string).collect()
}

#[test]
fn reads_every_returned_item_of_a_final_message_in_order() {
    let message = "The work of 1.1 and 1.2 is not accepted.\n\nRETURNED: 1.2\nRETURNED: 1.1\n\nSTATUS: SLEEPING\n";

    assert_eq!(returned(message), ["1.2", "1.1"]);
}

#[test]
fn reads_a_return_beside_the_status_line_through_markdown_markup_and_indentation() {
    let table = [
        ("RETURNED: 1.1\nSTATUS: SLEEPING", vec!["1.1"]),
        ("**RETURNED: 1.1**\n**STATUS:** SLEEPING", vec!["1.1"]),
        ("`RETURNED: 1.1`\nSTATUS: DONE", vec!["1.1"]),
        ("  RETURNED:   1.1   \nSTATUS: SLEEPING", vec!["1.1"]),
        ("RETURNED:1.1\nSTATUS: SLEEPING", vec!["1.1"]),
        ("text\r\nRETURNED: 1.1\r\nSTATUS: SLEEPING\r\n", vec!["1.1"]),
    ];
    for (message, expected) in table {
        assert_eq!(returned(message), expected, "{message:?}");
    }
}

#[test]
fn an_item_returned_twice_is_one_return_at_its_first_line() {
    let message = "RETURNED: 1.1\nRETURNED: 1.2\nRETURNED: 1.1\n";

    assert_eq!(returned(message), ["1.1", "1.2"]);
}

#[test]
fn a_message_without_the_line_returns_nothing() {
    let table = [
        // A lead of an older prompt: it returns the work in prose only.
        "The work is not accepted. Please fix the tests.\n\nSTATUS: SLEEPING\n",
        "STATUS: SLEEPING\n",
        "",
        // The status line alone is no return.
        "STATUS: DONE\n",
        // Another word that contains the line's word.
        "UNRETURNED: 1.1\nSTATUS: SLEEPING\n",
        // The word of a record of the status, not of a return.
        "STATUS: RETURNED\n",
    ];
    for message in table {
        assert_eq!(returned(message), Vec::<String>::new(), "{message:?}");
    }
}

#[test]
fn a_line_that_names_no_item_is_skipped_and_the_other_lines_are_read() {
    let table = [
        "RETURNED:\nRETURNED: 1.1\n",
        "RETURNED: the work of 1.1\nRETURNED: 1.1\n",
        "RETURNED 1.1\nRETURNED: 1.1\n",
        "RETURNED: 1\nRETURNED: 0.1\nRETURNED: 1.1\n",
        "RETURNED: 1.1 the seams\nRETURNED: 1.2\n",
    ];
    for (message, expected) in [
        (table[0], vec!["1.1"]),
        (table[1], vec!["1.1"]),
        (table[2], vec!["1.1"]),
        (table[3], vec!["1.1"]),
        (table[4], vec!["1.1", "1.2"]),
    ] {
        assert_eq!(returned(message), expected, "{message:?}");
    }
}
