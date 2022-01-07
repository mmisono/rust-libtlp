#![warn(rust_2018_idioms)]

use libtlp::{pci, DmaDirection, NetTlp};

use std::net::Ipv4Addr;

use anyhow::Result;
use structopt::StructOpt;

#[derive(StructOpt, Debug)]
#[structopt(name = "dma_read")]
struct Opt {
    /// Bus:Device.Function of NetTLP Adapter, "xx:xx.x"
    #[structopt(short = "b", long = "bdf")]
    bdf: pci::Bdf,

    /// Local address at NetTLP link
    #[structopt(short = "l", long = "local")]
    local_addr: Ipv4Addr,

    /// Remote address at NetTLP link
    #[structopt(short = "r", long = "remote")]
    remote_addr: Ipv4Addr,

    /// TLP tag
    #[structopt(short = "t", long = "tag", default_value = "0")]
    tag: u8,

    /// Target address
    #[structopt(
        short = "a",
        long = "addr",
        default_value = "0",
        parse(try_from_str = parse_int::parse)
    )]
    addr: u64,

    /// Transfer size (bytes)
    #[structopt(short = "s", long = "size", default_value = "4")]
    size: usize,

    /// MaxReadRequestSize
    #[structopt(short = "m", long = "mrrs", default_value = "512")]
    mrrs: usize,
}

fn main() -> Result<()> {
    let opt = Opt::from_args();
    let dir = DmaDirection::DmaIssuedByLibTLP;
    let nettlp = NetTlp::new(
        opt.bdf,
        opt.local_addr,
        opt.remote_addr,
        opt.tag,
        opt.mrrs,
        dir,
    )?;
    dbg!(&nettlp);

    let mut buf = vec![0; opt.size];
    nettlp.dma_read(opt.addr, &mut buf)?;
    dbg!(&buf);

    Ok(())
}
