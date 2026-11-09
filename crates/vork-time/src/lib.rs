//! Output-rate ABI, lazy frame/tick identities, and unbounded arithmetic for
//! consumed clocks. Ordinary animation evaluates numeric values.
use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{cmp::Ordering, fmt, str::FromStr};

pub mod exact;
mod clock;
pub use clock::{Clock, Request};

/// Reduced output rate at the integer output ABI. Derived times use `exact`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Time {
    num: i64,
    den: i64,
}

impl Time {
    pub const ZERO: Self = Self { num: 0, den: 1 };
    pub const ONE: Self = Self { num: 1, den: 1 };
    pub const fn integer(num: i64) -> Self {
        Self { num, den: 1 }
    }
    pub fn new(num: i64, den: i64) -> Result<Self> {
        Self::reduce(num.into(), den.into())
    }
    fn reduce(mut num: i128, mut den: i128) -> Result<Self> {
        ensure!(den != 0, "clock denominator must be nonzero");
        if den < 0 {
            num = num
                .checked_neg()
                .ok_or_else(|| anyhow::anyhow!("clock overflow"))?;
            den = den
                .checked_neg()
                .ok_or_else(|| anyhow::anyhow!("clock overflow"))?;
        }
        let gcd = gcd(num.unsigned_abs(), den as u128) as i128;
        Ok(Self {
            num: (num / gcd)
                .try_into()
                .map_err(|_| anyhow::anyhow!("clock numerator overflow"))?,
            den: (den / gcd)
                .try_into()
                .map_err(|_| anyhow::anyhow!("clock denominator overflow"))?,
        })
    }
    pub fn to_le_bytes(self) -> [u8; 16] {
        let mut bytes = [0; 16];
        bytes[..8].copy_from_slice(&self.num.to_le_bytes());
        bytes[8..].copy_from_slice(&self.den.to_le_bytes());
        bytes
    }
    pub fn numerator(self) -> i64 {
        self.num
    }
    pub fn denominator(self) -> i64 {
        self.den
    }
    pub fn as_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }
    pub fn as_f32(self) -> f32 {
        self.as_f64() as f32
    }
    pub fn from_f32(value: f32) -> Result<Self> {
        value.to_string().parse()
    }
    pub fn from_f64(value: f64) -> Result<Self> {
        value.to_string().parse()
    }
    /// Numbers retain their decimal value; repeating fractions remain rational strings.
    pub fn json(self) -> serde_json::Value {
        if let Some(decimal) = self.decimal() {
            if let Ok(number) = decimal.parse::<serde_json::Number>() {
                return number.into();
            }
        }
        self.to_string().into()
    }
    fn decimal(self) -> Option<String> {
        let mut den = self.den;
        for factor in [2, 5] {
            while den % factor == 0 {
                den /= factor;
            }
        }
        if den != 1 {
            return None;
        }
        let magnitude = i128::from(self.num).abs();
        let den = i128::from(self.den);
        let mut out = format!("{}{}", if self.num < 0 { "-" } else { "" }, magnitude / den);
        let mut remainder = magnitude % den;
        if remainder != 0 {
            out.push('.');
        }
        while remainder != 0 {
            remainder *= 10;
            out.push(char::from(b'0' + (remainder / den) as u8));
            remainder %= den;
        }
        Some(out)
    }
}
fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}
impl Default for Time {
    fn default() -> Self {
        Self::ZERO
    }
}
impl Ord for Time {
    fn cmp(&self, rhs: &Self) -> Ordering {
        (self.num as i128 * rhs.den as i128).cmp(&(rhs.num as i128 * self.den as i128))
    }
}
impl PartialOrd for Time {
    fn partial_cmp(&self, rhs: &Self) -> Option<Ordering> {
        Some(self.cmp(rhs))
    }
}
impl fmt::Display for Time {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.den == 1 {
            write!(f, "{}", self.num)
        } else {
            write!(f, "{}/{}", self.num, self.den)
        }
    }
}
impl FromStr for Time {
    type Err = anyhow::Error;
    fn from_str(value: &str) -> Result<Self> {
        let rate=exact::decimal(value)?;
        Ok(Self {
            num:rate.numer().try_into().map_err(|_|anyhow::anyhow!("output rate numerator exceeds i64 ABI"))?,
            den:rate.denom().try_into().map_err(|_|anyhow::anyhow!("output rate denominator exceeds i64 ABI"))?,
        })
    }
}

impl Serialize for Time {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        self.json().serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for Time {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let result = match value {
            serde_json::Value::Number(n) => n.to_string().parse(),
            serde_json::Value::String(s) if s.contains('/') => s.parse(),
            _ => bail_clock(),
        };
        result.map_err(serde::de::Error::custom)
    }
}
fn bail_clock() -> Result<Time> {
    bail!("clock must be a number or rational string")
}
impl schemars::JsonSchema for Time {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Time".into()
    }
    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({"x-clock":true,"description":"Output rate: decimal number or reduced rational string (frames per second).", "anyOf":[{"type":"number"},{"type":"string","pattern":"^-?[0-9]+/[1-9][0-9]*$"}]})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn output_rates_preserve_decimal_meaning_and_reduce_before_narrowing()->Result<()> {
        for text in ["29.97","29.970000000000001","0.00001","9007199254740993"] {
            let rate:Time=serde_json::from_str(text)?;
            assert_eq!(serde_json::from_value::<Time>(rate.json())?,rate);
        }
        assert_eq!("30000/1001".parse::<Time>()?,"60000/2002".parse()?);
        assert_ne!("29.97".parse::<Time>()?,"30000/1001".parse()?);
        let large=format!("1{}", "0".repeat(100));
        assert_eq!(format!("{large}/{large}").parse::<Time>()?,Time::ONE);
        for text in ["1/0","NaN","inf","+","-.","1e100","1e-100"] {assert!(text.parse::<Time>().is_err(),"{text}");}
        Ok(())
    }
    #[test]
    fn derived_frames_exceed_rate_pair_without_narrowing()->Result<()> {
        let rate=Time::new(1,i64::MAX)?;
        let time=Request::Frame(u32::MAX,rate).exact()?;
        assert!(time.value().numer().bits()>64);
        assert_eq!(time.scale(&exact::decimal(&rate.to_string())?).floor(),u32::MAX.into());
        Ok(())
    }
    #[test]
    fn rate_metadata_roundtrips_at_integer_limits()->Result<()> {
        for a in 0..=62 {for b in 0..=27 {
            let Some(den)=2i64.checked_pow(a).and_then(|n|5i64.checked_pow(b).and_then(|m|n.checked_mul(m))) else {continue};
            for num in [i64::MIN,i64::MAX,-1,0,1] {
                let rate=Time::new(num,den)?;
                assert_eq!(serde_json::from_value::<Time>(rate.json())?,rate);
            }
        }}
        Ok(())
    }
}
