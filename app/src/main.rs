use anyhow::Result;
use clap::Parser;
use resident_engine::shipyard::World;
use resident_engine::Args;

fn main() -> Result<()> {
    env_logger::init();

    let args = Args::parse();

    let world = World::new();

    resident_engine::run(args, world, |_world| {})
}
