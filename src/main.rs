mod execution;
mod hashing;
mod identity;
mod invocation;
mod reporting;
mod runner;
mod signals;
mod storage;

use clap::Parser;
use invocation::{Args, Invocation};

// By convention, commands like `timeout` use 125 for their own errors. Tests may return it too.
const WRAPPER_ERROR_EXIT_CODE: i32 = 125;

fn main() {
    let invocation = Invocation::from_args(Args::parse());
    if let Err(error) = invocation.and_then(runner::run).and_then(signals::exit) {
        eprintln!("prevtest: {error:#}");
        std::process::exit(WRAPPER_ERROR_EXIT_CODE);
    }
}
