use crate::error::Error;
use std::str::FromStr;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct Bdf {
    bus: u8,
    device: u8,
    func: u8,
}

impl Bdf {
    pub fn new(bus: u8, device: u8, func: u8) -> Self {
        debug_assert!(device < 32);
        debug_assert!(func < 8);
        Bdf { bus, device, func }
    }

    pub(crate) fn to_u16(self) -> u16 {
        ((self.bus as u16) << 8) | ((self.device as u16) << 3) | (self.func as u16)
    }
}

impl FromStr for Bdf {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // acceptable format: xx:xx.x
        lazy_static::lazy_static! {
            static ref RE: regex::Regex = regex::Regex::new(
                r"^\s*([[:xdigit:]]{2}):([[:xdigit:]]{2})\.([[:xdigit:]]{1})\s*$",
            )
            .unwrap();
        }

        RE.captures(s)
            .map(|caps| Bdf {
                bus: u8::from_str_radix(caps.get(1).unwrap().as_str(), 16).unwrap(),
                device: u8::from_str_radix(caps.get(2).unwrap().as_str(), 16).unwrap(),
                func: u8::from_str_radix(caps.get(3).unwrap().as_str(), 16).unwrap(),
            })
            .ok_or_else(|| Error::InvalidBDF(s.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn form_str() {
        let a = Bdf {
            bus: 0xff,
            device: 0x05,
            func: 0x1,
        };
        let b = Bdf::from_str("ff:05.1").unwrap();
        assert_eq!(a, b);
    }
}
