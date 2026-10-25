use std::fmt;

use thiserror::Error;

macro_rules! id_type {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(u64);

        impl $name {
            pub const fn from_raw(value: u64) -> Self {
                Self(value)
            }

            pub const fn raw(self) -> u64 {
                self.0
            }
        }

        impl From<u64> for $name {
            fn from(value: u64) -> Self {
                Self(value)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

id_type!(NodeId);
id_type!(EdgeId);
id_type!(ClipId);
id_type!(NoteId);
id_type!(SectionId);
id_type!(MarkerId);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PortId(u32);

impl PortId {
    pub const fn from_raw(value: u32) -> Self {
        Self(value)
    }

    pub const fn raw(self) -> u32 {
        self.0
    }
}

impl fmt::Display for PortId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AssetId([u8; 32]);

impl AssetId {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }

    pub fn to_hex(self) -> String {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        self.0
            .iter()
            .flat_map(|byte| {
                [
                    char::from(DIGITS[usize::from(byte >> 4)]),
                    char::from(DIGITS[usize::from(byte & 0x0f)]),
                ]
            })
            .collect()
    }

    pub fn from_hex(value: &str) -> Result<Self, AssetIdError> {
        let digits = value.as_bytes();
        if digits.len() != 64 {
            return Err(AssetIdError::Length);
        }
        let mut bytes = [0u8; 32];
        for (index, pair) in digits.chunks_exact(2).enumerate() {
            let high = hex_digit(pair[0])?;
            let low = hex_digit(pair[1])?;
            bytes[index] = (high << 4) | low;
        }
        Ok(Self(bytes))
    }
}

fn hex_digit(digit: u8) -> Result<u8, AssetIdError> {
    match digit {
        b'0'..=b'9' => Ok(digit - b'0'),
        b'a'..=b'f' => Ok(digit - b'a' + 10),
        b'A'..=b'F' => Ok(digit - b'A' + 10),
        _ => Err(AssetIdError::Digit),
    }
}

impl fmt::Debug for AssetId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "AssetId({})", self.to_hex())
    }
}

impl fmt::Display for AssetId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_hex())
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AssetIdError {
    #[error("asset id must be 64 hexadecimal characters")]
    Length,
    #[error("asset id contains a non-hexadecimal character")]
    Digit,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_id_hex_round_trips() {
        let bytes = [0xabu8; 32];
        let id = AssetId::from_bytes(bytes);
        assert_eq!(AssetId::from_hex(&id.to_hex()), Ok(id));
    }

    #[test]
    fn asset_id_rejects_short_input() {
        assert_eq!(AssetId::from_hex("abc"), Err(AssetIdError::Length));
    }

    #[test]
    fn asset_id_rejects_non_hex_input() {
        let value = "z".repeat(64);
        assert_eq!(AssetId::from_hex(&value), Err(AssetIdError::Digit));
    }
}
