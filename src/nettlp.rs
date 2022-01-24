use crate::error::Error;
use crate::pci;
use crate::tlp;

use std::net::Ipv4Addr;
use std::net::UdpSocket;

use bytes::buf::UninitSlice;
use bytes::BufMut;
use zerocopy::{AsBytes, FromBytes};

const EAGAIN: i32 = 11;

#[repr(packed)]
#[derive(Clone, Copy, Debug, AsBytes)]
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

#[derive(Copy, Clone, Debug)]
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
    // FIXME: Remove AsBytes trait bound.
    // We should create BytesMut with UninitSlice(*) instead of creating u8 slice.
    // but there is no BytesMut::from(UninitSlice) for now..
    // (* This is because a padding of a unpacked struct may be uninitialized.)
    pub fn dma_read_t<T: Sized + FromBytes + AsBytes>(
        &self,
        addr: u64,
        t: &mut T,
    ) -> Result<(), Error> {
        let ptr = (t as *mut T) as *mut u8;
        let len = std::mem::size_of::<T>();
        let mut slice = unsafe { std::slice::from_raw_parts_mut(ptr, len) };
        self.dma_read(addr, &mut slice, len)?;
        Ok(())
    }

    /// Read `len` bytes from a physical address `addr` into `buf`
    ///
    /// Several read requests are made when:
    ///   1. Read size is larger than MRRS
    ///   2. A request crosses 4k boundary
    // There is no BufMut::len(), and BufMut::remaining_mut() is not the buffer length.
    // It is the length that can be written from the current position.
    // For Vec<u8>, BufMut::remaining_mut() is isize::MAX - buf.len().
    // Therefore the function takes `len` as an additional argument.
    pub fn dma_read<T: BufMut>(&self, addr: u64, buf: &mut T, len: usize) -> Result<(), Error> {
        assert!(len <= buf.remaining_mut());
        let total_len = len;
        let mut p = addr;
        let mut received = 0;
        loop {
            let remain = total_len - received;
            let max_len = 0x1000 - (p & 0xFFF) as usize;
            let chunk_len = buf.chunk_mut().len();
            use std::cmp::min;
            let len = min(min(min(remain, self.mrrs), max_len), chunk_len);

            self.send_mrd(p, len)?;
            self.recv_cpld(p, &mut buf.chunk_mut()[..len])?;
            received += len;
            p += len as u64;
            unsafe {
                buf.advance_mut(len);
            }
            if received == total_len {
                break;
            }
        }
        Ok(())
    }

    fn send_mrd(&self, addr: u64, len: usize) -> Result<(), Error> {
        self.send_mr(addr, len, tlp::TlpType::Mrd, None)
    }

    fn send_mwr(&self, addr: u64, len: usize, data: &[u8]) -> Result<(), Error> {
        self.send_mr(addr, len, tlp::TlpType::Mwr, Some(data))
    }

    // Send a memory (reqd|write) request TLP with a nettlp header
    fn send_mr(
        &self,
        addr: u64,
        len: usize,
        t: tlp::TlpType,
        data: Option<&[u8]>,
    ) -> Result<(), Error> {
        let nh = NetTlpHdr::new();
        let mut packet = bytes::BytesMut::new();

        // NetTLP header
        packet.extend_from_slice(nh.as_bytes());

        // TLP header
        // Separte function calls are necessary to expolit generics
        if addr <= u32::MAX as u64 {
            let mh = tlp::TlpMrHdr::new(t, self.requester, self.tag, addr as u32, len);
            packet.extend_from_slice(mh.as_bytes());
        } else {
            let mh = tlp::TlpMrHdr::new(t, self.requester, self.tag, addr, len);
            packet.extend_from_slice(mh.as_bytes());
        };

        // Append data if any
        if let Some(data) = data {
            packet.extend_from_slice(data.as_bytes());
        }

        self.socket.send(&packet)?;
        Ok(())
    }

    // Receive completion with data TLP(s)
    // Note: It is possible to get several completion TLPs for one request
    // TODO: zero-copy
    fn recv_cpld(&self, addr: u64, buf: &mut UninitSlice) -> Result<(), Error> {
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
                return Err(Error::InvalidData(format!(
                    "Datagram size is less than TLP header size: {} < {}",
                    n,
                    nh_size + cpl_size
                )));
            }

            let cpld: tlp::TlpCplHdr =
                unsafe { std::ptr::read(recv_buf.as_ptr().add(nh_size) as *const _) };

            if !cpld.is_completion_with_data() {
                if cpld.is_completion() {
                    return Err(Error::InvalidAddress(addr));
                } else {
                    return Err(Error::InvalidData(format!(
                        "Invalid format type: {:#010b}",
                        cpld.fmt_type
                    )));
                };
            }
            if !cpld.is_valid_status() {
                return Err(Error::InvalidData(format!(
                    "Invalid status: {:#b}",
                    cpld.stcnt.to_be()
                )));
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

            if size > (n - (nh_size + cpl_size)) {
                dbg!("Corrupted TLP?", n, nh_size, cpl_size, size, cpld);
                return Err(Error::InvalidData(format!(
                    "TLP payload size is larger than the actual packet size: {} > {}",
                    size,
                    (n - (nh_size + cpl_size))
                )));
            }
            if size > buf_len {
                dbg!("BUG: buf is too small", size, buf_len, cpld);
                return Err(Error::InvalidData("Internal error".to_string()));
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

    /// DMA write
    pub fn dma_write(&self, addr: u64, buf: &[u8]) -> Result<(), Error> {
        assert!(
            addr & 0x3 == 0 && buf.len() % 4 == 0,
            "non DW-aligned requests are not implemented"
        );
        let total_len = buf.len();
        let mut p = addr;
        let mut sent = 0;
        loop {
            let remain = total_len - sent;
            let max_len = 0x1000 - (p & 0xFFF) as usize;
            use std::cmp::min;
            let len = min(min(remain, self.mrrs), max_len);
            let end = sent + len;

            self.send_mwr(p, len, &buf[sent..end])?;

            sent += len;
            p += len as u64;
            if sent >= total_len {
                break;
            }
        }
        Ok(())
    }

    /// Write `T` in a memory `addr`
    pub fn dma_write_t<T: Sized + AsBytes>(&self, addr: u64, t: T) -> Result<(), Error> {
        let ptr = (&t as *const T) as *const u8;
        let len = std::mem::size_of::<T>();
        let slice = unsafe { std::slice::from_raw_parts(ptr, len) };
        self.dma_write(addr, slice)?;
        Ok(())
    }
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
