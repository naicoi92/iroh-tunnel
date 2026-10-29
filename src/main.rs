//! All-in-one command-line entrypoint.

use clap::Parser;
use iroh_tunnel::cli::Cli;

fn main() {
    iroh_tunnel::dispatch::main(Cli::parse());
}
