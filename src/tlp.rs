use crate::pci;

// Some traits definitions for using u32 and u64 in generics

pub trait ToBe {
    fn to_be(&self) -> Self;
}

impl ToBe for u32 {
    fn to_be(&self) -> Self {
        u32::to_be(*self)
    }
}

impl ToBe for u64 {
    fn to_be(&self) -> Self {
        u64::to_be(*self)
    }
}

pub trait To64 {
    fn to_64(&self) -> u64;
}

impl To64 for u32 {
    fn to_64(&self) -> u64 {
        *self as u64
    }
}

impl To64 for u64 {
    fn to_64(&self) -> u64 {
        *self
    }
}

pub trait AlignDW {
    fn align_dw(&self) -> Self;
}

impl AlignDW for u32 {
    fn align_dw(&self) -> Self {
        self & !0x3
    }
}

impl AlignDW for u64 {
    fn align_dw(&self) -> Self {
        self & !0x3
    }
}

pub trait MaxValue {
    fn max_value(&self) -> u64;
}

impl MaxValue for u32 {
    fn max_value(&self) -> u64 {
        u32::MAX as u64
    }
}

impl MaxValue for u64 {
    fn max_value(&self) -> u64 {
        u64::MAX
    }
}

// NOTE:
// - TLP uses big endian
// - TLP is DW (4-bytes) aligned
// - References
//   - http://xillybus.com/tutorials/pci-express-tlp-pcie-primer-tutorial-guide-1
//   - https://community.mellanox.com/s/article/understanding-pcie-configuration-for-maximum-performance
//   - https://www.semisaga.com/2019/07/pcie-tlp-header-packet-formats-address.html

/// Memory Request Header (32bit address)
///
/// +---------------+---------------+---------------+---------------+
/// |       0       |       1       |       2       |       3       |
/// +---------------+---------------+---------------+---------------+
/// |7|6|5|4|3|2|1|0|7|6|5|4|3|2|1|0|7|6|5|4|3|2|1|0|7|6|5|4|3|2|1|0|
/// +---------------+---------------+---------------+---------------+
/// |R|Fmt|  Type   |R| TC  |   R   |T|E|Atr| R |      Length       |
/// +---------------+---------------+---------------+---------------+
/// |         Requeseter ID         |      Tag      | LastDW| 1stDW |
/// +---------------+---------------+---------------+---------------+
/// |                            Address                        | R |
/// +---------------+---------------+---------------+---------------+
///
/// or, 64bit address (4DW header)
/// +---------------+---------------+---------------+---------------+
/// |                            Address                            |
/// +---------------+---------------+---------------+---------------+
/// |                            Address                        | R |
/// +---------------+---------------+---------------+---------------+
///
///
///  FMT (3bit)
///     - 0?0 : 32bit Address
///     - 0?1 : 64bit Address
///
/// | TLP Type | Format    | Type   | Description          |
/// |----------|-----------|--------|----------------------|
/// | MR       | 000 / 001 | 0 0000 | Memory Read Request  |
/// | MW       | 010 / 011 | 0 0000 | Memory Write Request |
/// | Cpl      | 000       | 0 1010 | Completion w/o Data  |
/// | CplD     | 010       | 0 1010 | Completion w/ Data   |
///
// NOTE: For addresses below 4 GB, requesters must use the 32-bit format.
#[repr(packed)]
#[allow(dead_code)]
pub(crate) struct TlpMrHdr<T: ToBe + To64 + AlignDW + MaxValue> {
    // 1st DW
    /// Format and Type
    fmt_type: u8,
    /// Trafic Class
    tclass: u8,
    /// Flag, Attr, Reserved, Length
    falen: u16,

    // 2nd DW
    /// Requester ID
    requester: u16,
    /// Tag
    tag: u8,
    /// Last DW & 1st DW
    dw: u8,

    // 3rd DW (& 4th DW for 64bit address)
    /// Address
    addr: T,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum TlpType {
    /// Memory Read
    Mrd,
    /// Memory Write
    Mwr,
    _Unknown,
}

/// Completion Header
///
/// +---------------+---------------+---------------+---------------+
/// |       0       |       1       |       2       |       3       |
/// +---------------+---------------+---------------+---------------+
/// |7|6|5|4|3|2|1|0|7|6|5|4|3|2|1|0|7|6|5|4|3|2|1|0|7|6|5|4|3|2|1|0|
/// +---------------+---------------+---------------+---------------+
/// |R|Fmt|  Type   |R| TC  |   R   |T|E|Atr| R |      Length       |
/// +---------------+---------------+---------------+---------------+
/// |          Completer ID         |CplSt|B|      Byte Count       |
/// +---------------+---------------+---------------+---------------+
/// |          Requester ID         |      Tag      |R| Lower Addr  |
/// +---------------+---------------+---------------+---------------+
///
/// Length     : how many DWs are in this packet
/// Byte count : number of bytes left for transmission including in this packet
/// Lower addr : the 7 least significant bits of the address,
///              from which the first byte in this TLP was read
///
/// NOTE: data can be split into several completion TLPs
///
#[repr(packed)]
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
pub(crate) struct TlpCplHdr {
    // 1st DW
    /// Format and Type
    fmt_type: u8,
    /// Trafic Class
    tclass: u8,
    /// Flag, Attr, Reserved, Length
    falen: u16,

    // 2nd DW
    /// Completer ID
    completer: u16,
    /// Status & Byte count
    stcnt: u16,

    // 3rd DW
    /// Requester ID
    requester: u16,
    /// Tag
    tag: u8,
    /// Low address
    pub lowaddr: u8,
}

impl<T: ToBe + To64 + AlignDW + MaxValue> TlpMrHdr<T> {
    /// Create message request TLP
    pub(crate) fn new(
        tlp_type: TlpType,
        requester: pci::Bdf,
        tag: u8,
        addr: T,
        count: usize,
    ) -> Self {
        let addr64 = addr.max_value() > u32::MAX as u64;

        // Check if using 32bit addressing when appropriate
        if addr64 {
            debug_assert!(addr.to_64() > u32::MAX as u64);
        }

        let fmt_type: u8 = match tlp_type {
            TlpType::Mrd => {
                if addr64 {
                    0b0010_0000
                } else {
                    0b0000_0000
                }
            }
            TlpType::Mwr => {
                if addr64 {
                    0b0110_0000
                } else {
                    0b0100_0000
                }
            }
            _ => unimplemented!(),
        };

        let tclass: u8 = 0;
        let falen = calc_length(addr.to_64(), count as u64);
        let dw = calc_be(addr.to_64(), count as u64);

        TlpMrHdr {
            fmt_type: fmt_type.to_be(),
            tclass: tclass.to_be(),
            falen: falen.to_be(),
            requester: requester.to_u16().to_be(),
            tag: tag.to_be(),
            dw: dw.to_be(),
            addr: addr.align_dw().to_be(),
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) enum CplStatus {
    Success,
    Unsupported,
    ConfigurationRequestStatus,
    CompleterAbort,
    Unknown,
}

impl From<u16> for CplStatus {
    fn from(n: u16) -> CplStatus {
        match n {
            0x0000 => CplStatus::Success,
            0x2000 => CplStatus::Unsupported,
            0x4000 => CplStatus::ConfigurationRequestStatus,
            0x8000 => CplStatus::CompleterAbort,
            _ => CplStatus::Unknown,
        }
    }
}

const CPL_FMT_TYPE: u8 = 0b0100_1010;
const CPL_LENGTH_MASK: u16 = 0x03FF;
const CPL_COUNT_MASK: u16 = 0x0FFF;
const CPL_STATUS_MASK: u16 = 0xE000;
impl TlpCplHdr {
    pub(crate) fn is_valid_fmt_type(&self) -> bool {
        self.fmt_type == CPL_FMT_TYPE
    }

    pub(crate) fn is_valid_status(&self) -> bool {
        self.status() == CplStatus::Success
    }

    pub(crate) fn is_last_tlp(&self) -> bool {
        self.length() == (((self.lowaddr as u16 & 0x3) + self.count() + 3) >> 2)
    }

    pub(crate) fn status(&self) -> CplStatus {
        CplStatus::from(self.stcnt.to_be() & CPL_STATUS_MASK)
    }

    pub(crate) fn length(&self) -> u16 {
        self.falen.to_be() & CPL_LENGTH_MASK
    }

    pub(crate) fn count(&self) -> u16 {
        self.stcnt.to_be() & CPL_COUNT_MASK
    }
}

// Addresses used in TLP are DW (4byte) aligned.
// First and last BE (Byte enable) fields specifiy which of the four bytes are valid.
//
// Example:
//    addr: 0x3
//    count: 7
//                  0x0       0x4       0x8
//               |3|2|1|0| |3|2|1|0| |3|2|1|0|
//  valid data:         x   x x x x   x x
//
//  1st BE: 0001b
//  last BE: 1100b
//
fn calc_be(addr: u64, count: u64) -> u8 {
    let lastbe = calc_lastbe(addr, count);
    let firstbe = calc_firstbe(addr, count);
    (lastbe << 0x4) | firstbe
}

fn calc_lastbe(addr: u64, count: u64) -> u8 {
    let start = (addr >> 2) << 2;
    let end = addr + count;
    let end_start_ = if (end & 0x3) == 0 {
        end - 4
    } else {
        (end >> 2) << 2
    };
    let end_start = if end_start_ <= start {
        addr + 4
    } else {
        end_start_
    };
    if end < end_start {
        0
    } else {
        !(0xF << (end - end_start)) & 0xF
    }
}

fn calc_firstbe(addr: u64, count: u64) -> u8 {
    let be: u8 = if count < 4 {
        !(0xF << count) & 0xF
    } else {
        0xF
    };
    (be << (addr & 0x3)) & 0xF
}

// Calculate how many DWs are read / written
fn calc_length(addr: u64, count: u64) -> u16 {
    let start = addr & !0x3;
    let end = addr + count;
    let len = ((end - start) >> 2) as u16;
    if (end - start) & 0x3 > 0 {
        len + 1
    } else {
        len
    }
}
