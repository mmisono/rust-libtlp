use crate::error::Error;
use crate::pci;
use crate::tlp;

use std::net::Ipv4Addr;
use std::net::UdpSocket;

const EAGAIN: i32 = 11;

#[repr(packed)]
#[derive(Clone, Copy, Debug)]
struct NetTlpHdr {
    // NOTE: The header contants are not used for now
    /// Sequence number
    #[allow(dead_code)]
    seq: u16,
    /// Timestamp
    #[allow(dead_code)]
    timestamp: u32,
}

impl NetTlpHdr {
    fn new() -> Self {
        NetTlpHdr {
            seq: 0,
            timestamp: 0,
        }
    }
}

#[derive(Debug)]
pub enum DmaDirection {
    DmaIssuedByLibTLP,
    DmaIssuedByAdapter,
}

#[derive(Debug)]
pub struct NetTlp {
    pub remote_addr: Ipv4Addr,
    pub local_addr: Ipv4Addr,
    pub requester: pci::Bdf,
    pub tag: u8,
    pub mrrs: usize,
    pub dir: DmaDirection,
    pub socket: UdpSocket,
}

impl NetTlp {
    /* TODO: implement
    /// Port for messaging API
    const NETTLP_MSG_PORT: u16 = 0x2FFF; // 12287
    */
    /// Base port for DmaIssuedByLibTLP mode
    const NETTLP_LIBTLP_PORT_BASE: u16 = 0x3000;
    /// Base port for DmaIssuedByAdapter mode
    const NETTLP_ADAPTER_PORT_BASE: u16 = 0x4000;
    /// The timeout value of receiving completion TLPs
    const LIBTLP_CPL_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(500);

    pub fn new(
        bdf: pci::Bdf,
        local_addr: Ipv4Addr,
        remote_addr: Ipv4Addr,
        tag: u8,
        mrrs: usize,
        dir: DmaDirection,
    ) -> Result<Self, Error> {
        let requester = bdf;
        let port = match dir {
            DmaDirection::DmaIssuedByLibTLP => NetTlp::NETTLP_LIBTLP_PORT_BASE + (tag as u16),
            DmaDirection::DmaIssuedByAdapter => {
                NetTlp::NETTLP_ADAPTER_PORT_BASE + ((tag & 0x0F) as u16)
            }
        };
        let socket = UdpSocket::bind((local_addr, port))?;
        socket.set_read_timeout(Some(NetTlp::LIBTLP_CPL_TIMEOUT))?;
        socket.connect((remote_addr, port))?;
        Ok(NetTlp {
            remote_addr,
            local_addr,
            requester,
            tag,
            mrrs,
            dir,
            socket,
        })
    }

    /// Read `sizeof(T)` bytes into `t` from a physical addr
    ///
    /// # Safety
    ///
    /// This function should not be called with an unpacked type
    pub unsafe fn dma_read_t<T: Sized>(&self, addr: u64, t: &mut T) -> Result<(), Error> {
        self.dma_read(addr, as_u8_mut_slice(t))
    }

    /// Read `buf.len()` bytes from a physical addr
    ///
    /// Several read requests are made when:
    ///   1. Read size is larger than MRRS
    ///   2. A request crosses 4k boundary
    // TODO: zero-copy
    pub fn dma_read(&self, addr: u64, buf: &mut [u8]) -> Result<(), Error> {
        if buf.is_empty() {
            return Ok(());
        }
        let mut p = addr;
        let mut received = 0;
        loop {
            let start = received;
            let remain = buf[start..].len();
            let mut len = if remain > self.mrrs {
                self.mrrs
            } else {
                remain
            };
            // split requests if it crosses 4k boundary
            if (p & !0xFFF) != ((p + len as u64) & !0xFFF) {
                len = 0x1000 - (p & 0xFFF) as usize;
            }
            let end = start + len;

            self.send_mr(p, len)?;
            self.recv_cpld(&mut buf[start..end])?;
            received += len;
            p += len as u64;
            if received >= buf.len() {
                break;
            }
        }
        assert!(received == buf.len());

        Ok(())
    }

    // This function does not consider MRRS and boundary
    // Just for an experiment purpose
    pub fn _dma_read_vanilla(&self, addr: u64, buf: &mut [u8]) -> Result<(), Error> {
        if buf.is_empty() {
            return Ok(());
        }
        self.send_mr(addr, buf.len())?;
        self.recv_cpld(buf)
    }

    // Send a memory read request TLP with a nettlp header
    fn send_mr(&self, addr: u64, len: usize) -> Result<(), Error> {
        let nh = NetTlpHdr::new();
        let t = tlp::TlpType::Mrd;
        let mut packet = bytes::BytesMut::new();

        // It is safe to convert a packed struct to u8 slice
        packet.extend_from_slice(unsafe { as_u8_slice(&nh) });

        // Separte function calls are necessary to expolit generics
        if addr <= u32::MAX as u64 {
            let mh = tlp::TlpMrHdr::new(t, self.requester, self.tag, addr as u32, len);
            packet.extend_from_slice(unsafe { as_u8_slice(&mh) });
        } else {
            let mh = tlp::TlpMrHdr::new(t, self.requester, self.tag, addr, len);
            packet.extend_from_slice(unsafe { as_u8_slice(&mh) });
        };

        self.socket.send(&packet)?;
        Ok(())
    }

    // Receive completion with data TLP(s)
    // It is possible to get several completion TLPs for one request
    fn recv_cpld(&self, buf: &mut [u8]) -> Result<(), Error> {
        let nh_size = std::mem::size_of::<NetTlpHdr>();
        let cpl_size = std::mem::size_of::<tlp::TlpCplHdr>();
        // Extra bytes are for non DW-aligned data
        // For exmaple, when reading 7 bytes from 0x3,
        // the completion TLP contains 3*4 bytes data
        //
        //                  0x0       0x4       0x8
        //               |3|2|1|0| |3|2|1|0| |3|2|1|0|
        //  valid data:         x   x x x x   x x
        //
        let etra_bytes = 6; // just enough size
        let bufsize = nh_size + cpl_size + buf.len() + etra_bytes;
        let mut recv_buf = vec![0; bufsize];
        let invdataerr = Err(Error::from(std::io::Error::from(
            std::io::ErrorKind::InvalidData,
        )));
        let mut received = 0;
        loop {
            let n = self.socket.recv(&mut recv_buf).map_err(|e| {
                if errno::errno().0 == EAGAIN {
                    Error::Timeout
                } else {
                    Error::from(e)
                }
            })?;

            if n < nh_size + cpl_size {
                return invdataerr;
            }

            let cpld: tlp::TlpCplHdr =
                unsafe { std::ptr::read(recv_buf.as_ptr().add(nh_size) as *const _) };

            if !cpld.is_valid_fmt_type() || !cpld.is_valid_status() {
                return invdataerr;
            }

            let offset = (cpld.lowaddr & 0x3) as usize;
            let start = nh_size + cpl_size + offset;
            let end = if cpld.count() <= cpld.length() * 4 {
                start + (cpld.count() as usize)
            } else {
                start + (cpld.length() as usize) * 4 - offset
            };
            let size = end - start;
            let buf_start = received;
            let buf_end = received + size;
            let buf_len = buf[buf_start..].len();
            if size > buf_len {
                dbg!(
                    "BUG: buf is too small!",
                    offset,
                    start,
                    end,
                    size,
                    buf_start,
                    buf_end,
                    buf_len,
                    received,
                    cpld.length(),
                    cpld.count(),
                    cpld,
                );
                return invdataerr;
            }
            let tmp = &recv_buf[start..end];
            buf[buf_start..buf_end].copy_from_slice(&recv_buf[start..end]);
            received += tmp.len();

            if cpld.is_last_tlp() {
                break;
            }
        }
        Ok(())
    }

    /*
    /// DMA write
    pub fn dma_write(&self, addr: usize, buf: &mut [u8]) -> Result<(), Error> {
        unimplemented!();
    }
    */
}

unsafe fn as_u8_slice<T: Sized>(p: &T) -> &[u8] {
    std::slice::from_raw_parts((p as *const T) as *const u8, std::mem::size_of::<T>())
}

unsafe fn as_u8_mut_slice<T: Sized>(p: &mut T) -> &mut [u8] {
    std::slice::from_raw_parts_mut((p as *mut T) as *mut u8, std::mem::size_of::<T>())
}

// for debug
#[allow(dead_code)]
fn dump_packet(p: &[u8], nettlp: bool) {
    let s = std::mem::size_of::<NetTlpHdr>();
    if nettlp {
        // print nettlp header
        for b in p[..s].iter() {
            print!("{:02x} ", b);
        }
        println!();
    }
    // print TLP
    let start = if nettlp { s } else { 0 };
    for (i, b) in p[start..].iter().enumerate() {
        if i != 0 && i % 8 == 0 {
            println!();
        }
        print!("{:02x} ", b);
    }
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn init() {
        let remote_addr = Ipv4Addr::new(127, 0, 0, 1);
        let local_addr = Ipv4Addr::new(127, 0, 0, 1);
        let bdf = pci::Bdf::from_str("01:00.0").unwrap();
        let dir = DmaDirection::DmaIssuedByLibTLP;
        let tag = 0;
        let mrrs = 512;
        let _ = NetTlp::new(bdf, local_addr, remote_addr, tag, mrrs, dir).unwrap();
    }
}
