#[cfg(test)]
mod tests {
    use std::{os::unix::process::CommandExt, process::Command};

    // exec keeps the shell in the test's process and lets us set its signal disposition.
    fn shell(script: &str) -> ! {
        panic!("{}", Command::new("/bin/sh").args(["-c", script]).exec())
    }

    #[test]
    fn dies_by_signal() {
        shell("kill -TERM $$")
    }

    #[test]
    fn ignores_term() {
        shell("trap '' TERM; echo $$ > sleeper.pid; exec sleep 15")
    }

    #[test]
    fn handles_term() {
        shell("trap 'echo handled >> cleanups; exit 0' TERM; sleep 15")
    }
}
