#![warn(rust_2018_idioms)]

use libtlp::{pci, DmaDirection, NetTlp};

use std::net::Ipv4Addr;

use anyhow::Result;
use clap::Parser;

#[derive(Parser, Debug)]
#[clap(about, version)]
struct Args {
    /// Bus:Device.Function of NetTLP Adapter, "xx:xx.x"
    #[clap(short, long)]
    bdf: pci::Bdf,

    /// Local address at NetTLP link
    #[clap(short, long = "local")]
    local_addr: Ipv4Addr,

    /// Remote address at NetTLP link
    #[clap(short, long = "remote")]
    remote_addr: Ipv4Addr,

    /// TLP tag
    #[clap(short, long, default_value_t = 0)]
    tag: u8,

    /// Target address
    #[clap(
        short, long, default_value_t = 0,
        parse(try_from_str = parse_int::parse)
    )]
    addr: u64,

    /// Transfer data
    #[clap(
        short, long, default_value_t = 0,
        parse(try_from_str = parse_int::parse)
     )]
    val: u32,

    /// MaxReadRequestSize
    #[clap(short, long, default_value_t = 512)]
    mrrs: usize,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let dir = DmaDirection::DmaIssuedByLibTLP;
    let nettlp = NetTlp::new(
        args.bdf,
        args.local_addr,
        args.remote_addr,
        args.tag,
        args.mrrs,
        dir,
    )?;
    dbg!(&args);
    dbg!(&nettlp);

    nettlp.dma_write_t(args.addr, args.val)?;

    Ok(())
}
