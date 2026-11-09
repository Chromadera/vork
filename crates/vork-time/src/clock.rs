//! Authored seconds retain their spelling until a discrete consumer needs it.
use anyhow::{Result, ensure};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use num_traits::ToPrimitive;
use crate::exact::{Rational, decimal};

#[derive(Clone, Debug, PartialEq)]
pub struct Clock {
    spelling: std::sync::Arc<str>,
    numeric: f64,
    binary: bool,
}
impl Clock {
    pub fn exact(&self) -> Rational { if self.binary {Rational::from_float(self.numeric).expect("finite buffer clock")}else{decimal(&self.spelling).expect("validated clock")} }
    pub fn binary(value:f32)->Self {Self {spelling:value.to_string().into(),numeric:f64::from(value),binary:true}}
    pub fn as_f64(&self) -> f64 { self.numeric }
    pub fn as_f32(&self) -> f32 { self.numeric as f32 }
}
impl std::str::FromStr for Clock {
    type Err = anyhow::Error;
    fn from_str(text: &str) -> Result<Self> {
        let numeric = if text.contains('/') {decimal(text)?.to_f64().unwrap_or(f64::INFINITY)} else {
            let number:serde_json::Number = text.parse()?;
            number.as_f64().unwrap_or(f64::INFINITY)
        };
        Ok(Self { spelling: text.into(), numeric, binary:false })
    }
}
impl From<f64> for Clock {
    fn from(value: f64) -> Self { value.to_string().parse().expect("finite clock") }
}
impl From<f32> for Clock {
    fn from(value: f32) -> Self { value.to_string().parse().expect("finite clock") }
}
impl Serialize for Clock {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if self.binary {let value=self.exact(); return serializer.serialize_str(&format!("{}/{}",value.numer(),value.denom()))}
        if self.spelling.contains('/') { serializer.serialize_str(&self.spelling) }
        else { self.spelling.parse::<serde_json::Number>().map_err(serde::ser::Error::custom)?.serialize(serializer) }
    }
}
impl<'de> Deserialize<'de> for Clock {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let text = match value {
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::String(s) if s.contains('/') => s,
            _ => return Err(serde::de::Error::custom("clock must be numeric seconds or a rational string")),
        };
        text.parse().map_err(serde::de::Error::custom)
    }
}
impl schemars::JsonSchema for Clock {
    fn schema_name() -> std::borrow::Cow<'static, str> { "Clock".into() }
    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({"x-clock":true,"anyOf":[{"type":"number"},{"type":"string","pattern":"^-?[0-9]+/[1-9][0-9]*$"}]})
    }
}

/// Internal request identity. Ordinary evaluation lowers directly; only a
/// discrete consumer materializes unbounded arithmetic.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    Frame(u32, crate::Time),
    Tick(u32, Clock, Clock),
    Authored(Clock),
    Numeric(f32),
    Exact(crate::exact::Germ),
}
impl Request {
    pub fn numeric(&self) -> Result<f32> {
        let value = match self {
            Self::Frame(n, fps) => frame_numeric(*n,*fps)?,
            Self::Tick(..) => return self.exact()?.numeric(),
            Self::Authored(t) => return crate::exact::Germ::point(t.exact()).numeric(),
            Self::Numeric(t) => *t,
            Self::Exact(t) => return t.numeric(),
        };
        ensure!(value.is_finite(), "time exceeds finite numeric evaluation range");
        Ok(value)
    }
    pub fn exact(&self) -> Result<crate::exact::Germ> {
        use crate::exact::{Germ, ieee};
        Ok(match self {
            Self::Frame(n, fps) => Germ::point(Rational::new(num_bigint::BigInt::from(*n) * fps.denominator(), fps.numerator().into())),
            Self::Tick(k,start,dt) => Germ::point(start.exact()+Rational::from_integer((*k).into())*dt.exact()),
            Self::Authored(t) => Germ::point(t.exact()),
            Self::Numeric(t) => Germ::point(ieee(*t)?),
            Self::Exact(t) => t.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_frame_lowering_matches_unbounded_rounding() -> Result<()> {
        for rate in ["24", "30000/1001", "60000/1001", "1/9223372036854775807", "9223372036854775807"] {
            let rate=rate.parse()?;
            for n in [0,1,3,7,107892,16777217,u32::MAX] {
                let request=Request::Frame(n,rate);
                assert_eq!(request.numeric()?.to_bits(),request.exact()?.numeric()?.to_bits(),"{n} at {rate}");
            }
        }
        for value in [0.,1.,0.1,-1.] {
            let clock=Clock::binary(value);
            assert_eq!(serde_json::from_str::<Clock>(&serde_json::to_string(&clock)?)?.exact(),clock.exact());
        }
        Ok(())
    }
    #[test]
    fn authored_clock_roundtrips_without_binary_rounding() -> Result<()> {
        for text in ["0.016666666666666666", "1e-100", "\"1/120\"", "3600"] {
            let clock: Clock=serde_json::from_str(text)?;
            assert_eq!(serde_json::from_str::<Clock>(&serde_json::to_string(&clock)?)?.exact(),clock.exact());
        }
        let fps:crate::Time="30000/1001".parse()?;
        let clock:Clock=serde_json::from_str("3600")?;
        let count=clock.exact()*decimal(&fps.to_string())?;
        assert_eq!(count.ceil().to_integer(),107893.into());
        for n in [1,5,7,10,17,107892,u32::MAX] {
            let request=Request::Frame(n,fps);
            assert_eq!(request.exact()?.scale(&decimal(&fps.to_string())?).floor(),n.into());
        }
        Ok(())
    }
}

impl Request {
    pub fn previous(&self,fps:crate::Time)->Result<Self> {
        if let Self::Frame(n,rate)=self {
            if rate==&fps {return Ok(Self::Frame(n.saturating_sub(1),fps))}
        }
        let value=self.exact()?.sub(&crate::exact::Germ::point(decimal(&fps.to_string())?.recip()));
        Ok(Self::Exact(if value.compare_point(&Rational::from_integer(0.into())).is_lt() {
            crate::exact::Germ::point(Rational::from_integer(0.into()))
        }else{value}))
    }
}

impl From<f32> for Request { fn from(time:f32)->Self {Self::Numeric(time)} }
impl From<&Request> for Request { fn from(time:&Request)->Self {time.clone()} }
impl From<&Clock> for Request { fn from(time:&Clock)->Self {Self::Authored(time.clone())} }

/// The frame ABI bounds n to u32 and rate terms to positive i64. Products
/// therefore use at most 95 bits. Scaling to a 24-bit significand uses at most
/// max(95, 63+24) bits; u128 arithmetic cannot overflow here.
fn frame_numeric(index:u32,rate:crate::Time)->Result<f32> {
    ensure!(rate>crate::Time::ZERO,"frame rate must be positive");
    if index==0 {return Ok(0.)}
    let mut n=u128::from(index)*rate.denominator() as u128;
    let mut d=rate.numerator() as u128;
    let mut exponent=n.ilog2() as i32-d.ilog2() as i32;
    if if exponent>=0 {n<(d<<exponent)}else{(n<<(-exponent))<d} {exponent-=1;}
    let shift=23-exponent;
    if shift>=0 {n<<=shift;}else{d<<=-shift;}
    let mut significand=n/d;
    let remainder=n%d;
    if remainder*2>d || (remainder*2==d && significand&1!=0) {significand+=1;}
    if significand==1<<24 {significand>>=1;exponent+=1;}
    Ok(f32::from_bits(((exponent+127) as u32)<<23 | (significand as u32&0x7fffff)))
}
