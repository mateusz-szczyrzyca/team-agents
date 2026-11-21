//! The command-check shell is a valid, infallibly constructed program.

use agent_core::config::Program;

#[test]
fn posix_shell_is_the_bare_sh_program() {
    let shell = Program::posix_shell();
    assert_eq!(shell, "sh".parse::<Program>().expect("valid shell"));
    assert!(shell.is_bare_name());
}
