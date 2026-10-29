//! Standalone Access command-line entrypoint.

use clap::Parser;
use iroh_tunnel::cli::AccessCli;

fn main() {
    iroh_tunnel::dispatch::main(AccessCli::parse().into());
}
