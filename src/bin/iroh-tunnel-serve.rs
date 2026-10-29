//! Standalone Serve command-line entrypoint.

use clap::Parser;
use iroh_tunnel::cli::ServeCli;

fn main() {
    iroh_tunnel::dispatch::main(ServeCli::parse().into());
}
