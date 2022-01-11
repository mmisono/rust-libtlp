#![warn(rust_2018_idioms)]

use libtlp::{pci, DmaDirection, NetTlp};

use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;

use anyhow::{bail, Result};
use clap::Parser;

static RUNNING: AtomicBool = AtomicBool::new(true);

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
        long, default_value_t = 0,
        parse(try_from_str = parse_int::parse)
    )]
    region_addr: u64,

    /// DMA region size
    #[clap(long, default_value_t = 8*1024*1024)]
    region_size: usize,

    /// DMA length for one request
    #[clap(long, default_value_t = 256)]
    dma_len: usize,

    /// DMA Pattern
    #[clap(long, default_value = "seq")]
    pattern: DmaPattern,

    /// MaxReadRequestSize (MRRS)
    #[clap(short, long, default_value_t = 512)]
    mrrs: usize,

    /// Measure latency
    #[clap(long)]
    latency: bool,

    /// Number of threads
    #[clap(long, default_value_t = 1)]
    nthreads: u8,

    /// Count of itertaions for benchmark
    #[clap(long, default_value_t = 0)]
    count: u32,

    /// Interval between iterations (ms)
    #[clap(long, default_value_t = 0)]
    interval: u64,

    /// Benchmark duration
    #[clap(long, default_value_t = 0)]
    duration: u32,

    /// Debug mode
    #[clap(short, long)]
    debug: bool,
}

#[derive(Copy, Clone, Debug)]
enum DmaPattern {
    SEQ,
    SEQ512,
    FIX,
    RANDOM,
}

impl std::str::FromStr for DmaPattern {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let pat = match s {
            "seq" => DmaPattern::SEQ,
            "seq512" => DmaPattern::SEQ512,
            "fix" => DmaPattern::FIX,
            "random" => DmaPattern::RANDOM,
            _ => bail!("Invalid pattern: {}", s),
        };
        Ok(pat)
    }
}

fn next_addr(start: u64, size: u64, addr: u64, len: u64, pat: DmaPattern) -> u64 {
    match pat {
        DmaPattern::SEQ => {
            if (addr + len) > (start + size) {
                start
            } else {
                addr + len
            }
        }
        DmaPattern::SEQ512 => {
            if (addr + 512) > (start + size) {
                start
            } else {
                addr + 512
            }
        }
        DmaPattern::FIX => addr,
        DmaPattern::RANDOM => start + (rand::random::<u64>() % (size - len)) & !0xFFF,
    }
}

#[derive(Debug)]
#[allow(dead_code)]
struct ThreadParam {
    cpu: u8,
    region_addr: u64,
    region_size: usize,
    dma_len: usize,
    mrrs: usize,
    count: u32,
    interval: u64,
    latency: bool,
    dir: DmaDirection,
    pattern: DmaPattern,
    ntrans: Arc<AtomicU64>,
    nbytes: Arc<AtomicU64>,
}

// TODO: Support DMA write
fn bench_thread(nettlp: NetTlp, param: ThreadParam) {
    let cores = [param.cpu as usize];
    affinity::set_thread_affinity(&cores).unwrap();

    let mut count = 0;
    let len = param.dma_len;
    let mut buf = bytes::BytesMut::with_capacity(len);
    let mut addr = param.region_addr;

    println!(
        "start on cpu {}, address {:#x}, size {}, dma_len {}, mrrs {}",
        param.cpu, param.region_addr, param.region_size, len, param.mrrs
    );

    loop {
        if !RUNNING.load(Ordering::SeqCst) {
            break;
        }

        buf.clear();

        if param.latency {
            let now = std::time::SystemTime::now();
            nettlp.dma_read(addr, &mut buf, len).unwrap();
            println!(
                "latency: cpu on {}, {} nsec",
                param.cpu,
                now.elapsed().unwrap().as_nanos()
            );
        } else {
            nettlp.dma_read(addr, &mut buf, len).unwrap();
        }
        param.ntrans.fetch_add(1, Ordering::SeqCst);
        param.nbytes.fetch_add(len as u64, Ordering::SeqCst);
        count += 1;

        addr = next_addr(
            param.region_addr,
            param.region_size as u64,
            addr,
            len as u64,
            param.pattern,
        );

        if param.count > 0 && count >= param.count {
            RUNNING.store(false, Ordering::SeqCst);
            break;
        }

        if param.interval > 0 {
            std::thread::sleep(std::time::Duration::from_millis(param.interval));
        }
    }
}

fn count_thread(ntrans: Vec<Arc<AtomicU64>>, nbytes: Vec<Arc<AtomicU64>>, duration: u32) {
    // run this thread on the last cpu
    let cores = [affinity::get_core_num() - 1];
    affinity::set_thread_affinity(&cores).unwrap();

    println!("start count thread on {}", cores[0]);

    let mut count = 0;

    loop {
        if !RUNNING.load(Ordering::SeqCst) {
            break;
        }

        fn collect(v: &Vec<Arc<AtomicU64>>) -> Vec<u64> {
            v.iter().map(|x| x.load(Ordering::SeqCst)).collect()
        }

        let before_ntrans: Vec<u64> = collect(&ntrans);
        let before_nbytes: Vec<u64> = collect(&nbytes);

        std::thread::sleep(std::time::Duration::from_secs(1));
        if !RUNNING.load(Ordering::SeqCst) {
            break;
        }

        let after_ntrans: Vec<u64> = collect(&ntrans);
        let after_nbytes: Vec<u64> = collect(&nbytes);

        fn diff_sum(before: &[u64], after: &[u64]) -> u64 {
            before.iter().zip(after.iter()).map(|(x, y)| y - x).sum()
        }

        let ntrans: u64 = diff_sum(&before_ntrans, &after_ntrans);
        let nbytes: u64 = diff_sum(&before_nbytes, &after_nbytes);

        println!("{}: {} bps, {} tps", count, nbytes * 8, ntrans);

        count += 1;
        if duration > 0 && count >= duration {
            RUNNING.store(false, Ordering::SeqCst);
            break;
        }
    }
}

fn benchmark(args: &Args) -> Result<()> {
    let mut threads = vec![];
    let mut ntrans_ = vec![];
    let mut nbytes_ = vec![];
    let duration = args.duration;

    for n in 0..args.nthreads {
        let cpu = n;
        let tag = n;
        let region_size = args.region_size / (args.nthreads as usize);
        let region_addr = args.region_addr + (region_size * n as usize) as u64;
        let dir = DmaDirection::DmaIssuedByLibTLP;
        let ntrans = Arc::new(AtomicU64::new(0));
        let ntrans_clone = Arc::clone(&ntrans);
        ntrans_.push(ntrans);
        let nbytes = Arc::new(AtomicU64::new(0));
        let nbytes_clone = Arc::clone(&nbytes);
        nbytes_.push(nbytes);
        let nettlp = NetTlp::new(
            args.bdf,
            args.local_addr,
            args.remote_addr,
            tag,
            args.mrrs,
            DmaDirection::DmaIssuedByLibTLP,
        )?;
        let param = ThreadParam {
            cpu,
            region_addr,
            region_size,
            dma_len: args.dma_len,
            mrrs: args.mrrs,
            count: args.count,
            latency: args.latency,
            interval: args.interval,
            dir,
            pattern: args.pattern,
            ntrans: ntrans_clone,
            nbytes: nbytes_clone,
        };
        threads.push(thread::spawn(move || bench_thread(nettlp, param)));
    }

    threads.push(thread::spawn(move || {
        count_thread(ntrans_, nbytes_, duration)
    }));

    for th in threads {
        th.join().unwrap();
    }

    Ok(())
}

fn main() -> Result<()> {
    let args = Args::parse();

    ctrlc::set_handler(|| {
        println!("Received Ctrl-C, quitting...");
        RUNNING.store(false, Ordering::SeqCst);
    })?;

    benchmark(&args)?;
    Ok(())
}
