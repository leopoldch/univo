//! The few `safetensors` checks and reads a static model needs: an 8-byte
//! header length, a JSON header, then tensors that tile the data exactly.

use std::collections::BTreeMap;

/// Element types a static model may store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dtype {
    F64,
    F32,
    F16,
    I64,
    I32,
    I8,
    Other,
}

impl Dtype {
    fn parse(name: &str) -> Self {
        match name {
            "F64" => Self::F64,
            "F32" => Self::F32,
            "F16" => Self::F16,
            "I64" => Self::I64,
            "I32" => Self::I32,
            "I8" => Self::I8,
            _ => Self::Other,
        }
    }

    /// Bytes per element, `None` for a type no static model uses.
    fn size(self) -> Option<usize> {
        match self {
            Self::F64 | Self::I64 => Some(8),
            Self::F32 | Self::I32 => Some(4),
            Self::F16 => Some(2),
            Self::I8 => Some(1),
            Self::Other => None,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Tensor {
    pub(crate) dtype: Dtype,
    pub(crate) shape: Vec<usize>,
    /// Absolute byte range in the file.
    pub(crate) start: usize,
    pub(crate) end: usize,
}

#[derive(serde::Deserialize)]
struct Entry {
    dtype: String,
    shape: Vec<usize>,
    data_offsets: (usize, usize),
}

/// Same bound as the `safetensors` crate: a larger header is refused.
const MAX_HEADER: usize = 100_000_000;

/// Tensors by name.
///
/// # Errors
///
/// Returns a description when the header is malformed, a tensor's size
/// disagrees with its shape, or the tensors do not tile the data.
pub(crate) fn parse(bytes: &[u8]) -> Result<BTreeMap<String, Tensor>, String> {
    let length: [u8; 8] = bytes
        .get(..8)
        .and_then(|head| head.try_into().ok())
        .ok_or("the file is shorter than its header length")?;
    let length = usize::try_from(u64::from_le_bytes(length))
        .ok()
        .filter(|&length| length <= MAX_HEADER)
        .ok_or("the header is too large")?;
    let data_start = 8usize
        .checked_add(length)
        .ok_or("the header is too large")?;
    let header = bytes
        .get(8..data_start)
        .ok_or("the file is shorter than its header")?;
    let mut entries: BTreeMap<String, serde_json::Value> =
        serde_json::from_slice(header).map_err(|error| format!("invalid header: {error}"))?;
    entries.remove("__metadata__");

    let mut tensors = BTreeMap::new();
    for (name, value) in entries {
        let entry: Entry = serde_json::from_value(value)
            .map_err(|error| format!("invalid header entry {name}: {error}"))?;
        let dtype = Dtype::parse(&entry.dtype);
        let (start, end) = entry.data_offsets;
        let elements = entry
            .shape
            .iter()
            .try_fold(1usize, |count, &extent| count.checked_mul(extent));
        let expected = dtype
            .size()
            .and_then(|size| elements.and_then(|count| count.checked_mul(size)));
        if end < start || expected.is_some_and(|bytes| bytes != end - start) {
            return Err(format!("tensor {name} does not match its shape"));
        }
        tensors.insert(
            name,
            Tensor {
                dtype,
                shape: entry.shape,
                start: data_start.saturating_add(start),
                end: data_start.saturating_add(end),
            },
        );
    }

    let mut ranges: Vec<_> = tensors.values().map(|t| (t.start, t.end)).collect();
    ranges.sort_unstable();
    let mut covered = data_start;
    for (start, end) in ranges {
        if start != covered {
            return Err("tensors overlap or leave gaps".to_owned());
        }
        covered = end;
    }
    if covered != bytes.len() {
        return Err("the data does not match the tensors".to_owned());
    }
    Ok(tensors)
}

/// The value of an IEEE 754 half-precision number.
pub(crate) fn f16_to_f32(bits: u16) -> f32 {
    let sign = u32::from(bits >> 15) << 31;
    let exponent = u32::from((bits >> 10) & 0x1f);
    let mantissa = u32::from(bits & 0x3ff);
    let magnitude = match (exponent, mantissa) {
        (0, 0) => 0,
        // Subnormal: the value is mantissa · 2^-24, exact in f32.
        (0, _) => {
            let shift = mantissa.leading_zeros() - 21;
            let mantissa = (mantissa << shift) & 0x3ff;
            ((113 - shift) << 23) | (mantissa << 13)
        }
        (0x1f, _) => 0x7f80_0000 | (mantissa << 13),
        _ => ((exponent + 112) << 23) | (mantissa << 13),
    };
    f32::from_bits(sign | magnitude)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(header: &str, data: &[u8]) -> Vec<u8> {
        let mut bytes = (header.len() as u64).to_le_bytes().to_vec();
        bytes.extend_from_slice(header.as_bytes());
        bytes.extend_from_slice(data);
        bytes
    }

    #[test]
    fn reads_tensors_that_tile_the_data() {
        let bytes = file(
            r#"{"__metadata__":{"a":"b"},"e":{"dtype":"I8","shape":[2,3],"data_offsets":[0,6]},"w":{"dtype":"F32","shape":[1],"data_offsets":[6,10]}}"#,
            &[0; 10],
        );
        let tensors = parse(&bytes).unwrap();
        assert_eq!(tensors["e"].shape, [2, 3]);
        assert_eq!(tensors["e"].dtype, Dtype::I8);
        assert_eq!(tensors["w"].end, bytes.len());
    }

    #[test]
    fn refuses_inconsistent_files() {
        for (header, data) in [
            (
                r#"{"e":{"dtype":"I8","shape":[2,3],"data_offsets":[0,5]}}"#,
                5,
            ),
            (
                r#"{"e":{"dtype":"I8","shape":[2],"data_offsets":[1,3]}}"#,
                3,
            ),
            (
                r#"{"e":{"dtype":"I8","shape":[2],"data_offsets":[0,2]}}"#,
                3,
            ),
            (
                r#"{"e":{"dtype":"I8","shape":[2],"data_offsets":[2,0]}}"#,
                2,
            ),
            (r#"{"e":{"dtype":"I8","shape":[2]}}"#, 2),
            ("not json", 0),
        ] {
            assert!(parse(&file(header, &vec![0; data])).is_err(), "{header}");
        }
        assert!(parse(&[1, 0]).is_err());
        assert!(parse(&u64::MAX.to_le_bytes()).is_err());
    }

    #[test]
    fn converts_every_half_exactly() {
        for bits in 0..=u16::MAX {
            let value = f16_to_f32(bits);
            let sign = if bits >> 15 == 1 { -1.0f64 } else { 1.0 };
            let exponent = i32::from((bits >> 10) & 0x1f);
            let mantissa = f64::from(bits & 0x3ff);
            let expected = match exponent {
                0 => sign * mantissa * 2f64.powi(-24),
                0x1f if mantissa == 0.0 => sign * f64::INFINITY,
                0x1f => f64::NAN,
                _ => sign * (1.0 + mantissa / 1024.0) * 2f64.powi(exponent - 15),
            };
            if expected.is_nan() {
                assert!(value.is_nan());
            } else {
                assert_eq!(f64::from(value).to_bits(), expected.to_bits(), "{bits:#x}");
            }
        }
    }
}
