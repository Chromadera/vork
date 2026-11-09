//! Arithmetic for consumed time-valued channels and discrete timing decisions.
//! Ordinary numeric animation does not construct these values.
use anyhow::{Result, ensure};
use num_bigint::BigInt;
use num_integer::Integer;
use num_traits::{One, Signed, ToPrimitive, Zero};
use std::cmp::Ordering;

pub type Rational = num_rational::BigRational;

/// Decode authored decimal seconds without passing through binary floating point.
pub fn decimal(text: &str) -> Result<Rational> {
    if let Some((n, d)) = text.split_once('/') {
        let n: BigInt = n.parse()?;
        let d: BigInt = d.parse()?;
        ensure!(!d.is_zero(), "time denominator must be nonzero");
        return Ok(Rational::new(n, d));
    }
    let (mantissa, exponent) = text.split_once(['e', 'E']).unwrap_or((text, "0"));
    let exponent: BigInt = exponent.parse()?;
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    ensure!(!whole.is_empty() && fraction.bytes().all(|b| b.is_ascii_digit()), "invalid decimal time");
    let digits: BigInt = format!("{whole}{fraction}").parse()?;
    if digits.is_zero() { return Ok(Rational::zero()); }
    let scale = BigInt::from(fraction.len()) - exponent;
    let mut magnitude = scale.abs();
    let mut power = BigInt::one();
    let mut factor = BigInt::from(10);
    while !magnitude.is_zero() {
        if magnitude.is_odd() { power *= &factor; }
        magnitude >>= 1;
        if !magnitude.is_zero() { factor = &factor * &factor; }
    }
    Ok(if !scale.is_negative() { Rational::new(digits, power) } else { Rational::from_integer(digits * power) })
}

/// Numeric controls and decoded buffers mean their stored IEEE value.
pub fn ieee(value: f32) -> Result<Rational> {
    ensure!(value.is_finite(), "numeric animation must be finite");
    Ok(Rational::from_float(value).expect("finite IEEE value"))
}

/// Direct local polynomial arithmetic in positive epsilon. A point has only a
/// constant coefficient; an incoming left limit starts as t - epsilon. Keeping
/// coefficients through composition handles reversal, extrema and cancellation.
/// There is no expression tree and no chosen numerical epsilon.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Germ {
    coefficients: Vec<Rational>,
}

impl std::ops::Add for Germ {
    type Output = Self;
    fn add(self, rhs: Self) -> Self { Germ::add(&self, &rhs) }
}
impl std::ops::Sub for Germ {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self { Germ::sub(&self, &rhs) }
}
impl std::ops::Mul for Germ {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self { Germ::mul(&self, &rhs) }
}

impl Germ {
    pub fn point(value: Rational) -> Self { Self { coefficients: vec![value] } }
    pub fn left(value: Rational) -> Self {
        Self { coefficients: vec![value, -Rational::one()] }
    }
    pub fn right(value: Rational) -> Self {
        Self { coefficients: vec![value, Rational::one()] }
    }
    pub fn value(&self) -> &Rational { &self.coefficients[0] }
    fn normalized(mut coefficients: Vec<Rational>) -> Self {
        while coefficients.len() > 1 && coefficients.last().unwrap().is_zero() { coefficients.pop(); }
        Self { coefficients }
    }
    pub fn add(&self, rhs: &Self) -> Self {
        let mut result = self.coefficients.clone();
        result.resize(result.len().max(rhs.coefficients.len()), Rational::zero());
        for (out, value) in result.iter_mut().zip(&rhs.coefficients) { *out += value; }
        Self::normalized(result)
    }
    pub fn sub(&self, rhs: &Self) -> Self { self.add(&rhs.scale(&-Rational::one())) }
    pub fn scale(&self, rhs: &Rational) -> Self {
        Self::normalized(self.coefficients.iter().map(|value| value * rhs).collect())
    }
    pub fn mul(&self, rhs: &Self) -> Self {
        let mut result = vec![Rational::zero(); self.coefficients.len() + rhs.coefficients.len() - 1];
        for (i, a) in self.coefficients.iter().enumerate() {
            for (j, b) in rhs.coefficients.iter().enumerate() { result[i+j] += a * b; }
        }
        Self::normalized(result)
    }
    /// Compare the approached values; constant equality is inclusive.
    pub fn compare(&self, rhs: &Self) -> Ordering {
        for i in 0..self.coefficients.len().max(rhs.coefficients.len()) {
            let zero = Rational::zero();
            let order = self.coefficients.get(i).unwrap_or(&zero).cmp(rhs.coefficients.get(i).unwrap_or(&zero));
            if order != Ordering::Equal { return order; }
        }
        Ordering::Equal
    }
    pub fn compare_point(&self, rhs: &Rational) -> Ordering { self.compare(&Self::point(rhs.clone())) }
    pub fn floor(&self) -> BigInt {
        let value = self.value();
        let mut floor = value.numer().div_floor(value.denom());
        if value.is_integer() && self.compare_point(value) == Ordering::Less { floor -= 1; }
        floor
    }
    pub fn floor_clamped(&self, lower: i64, upper: i64) -> Result<i64> {
        ensure!(lower <= upper, "invalid time index bounds");
        Ok(self.floor().clamp(lower.into(), upper.into()).to_i64().unwrap())
    }
    pub fn euclidean_loop(&self, start: &Rational, end: &Rational) -> Result<Self> {
        let period = end - start;
        ensure!(period.is_positive(), "loop period must be positive");
        let offset = self.sub(&Self::point(start.clone()));
        let cycle = offset.scale(&period.recip()).floor();
        Ok(self.sub(&Self::point(period * Rational::from_integer(cycle))))
    }
    pub fn numeric(&self) -> Result<f32> {
        // ToPrimitive may pass through f64. Correct against exact IEEE cell
        // midpoints so an intermediate rounding cannot move a decision's input.
        let number=self.value();
        let mut value=number.to_f32().unwrap_or(if number.is_negative(){f32::NEG_INFINITY}else{f32::INFINITY});
        if !value.is_finite() {value=if number.is_negative(){-f32::MAX}else{f32::MAX};}
        loop {
            let lower=value.next_down();
            let upper=value.next_up();
            let midpoint=|a:f32,b:f32| -> Result<Rational> {Ok((ieee(a)?+ieee(b)?)/Rational::from_integer(2.into()))};
            if lower.is_finite() {
                let boundary=midpoint(lower,value)?;
                if number<&boundary || (number==&boundary && value.to_bits()&1!=0) {value=lower;continue}
            } else {
                let boundary=ieee(value)?-(ieee(upper)?-ieee(value)?)/Rational::from_integer(2.into());
                ensure!(number>&boundary,"time exceeds finite numeric evaluation range");
            }
            if upper.is_finite() {
                let boundary=midpoint(value,upper)?;
                if number>&boundary || (number==&boundary && value.to_bits()&1!=0) {value=upper;continue}
            } else {
                let boundary=ieee(value)?+(ieee(value)?-ieee(lower)?)/Rational::from_integer(2.into());
                ensure!(number<&boundary,"time exceeds finite numeric evaluation range");
            }
            return Ok(value)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn q(text: &str) -> Rational { decimal(text).unwrap() }
    #[test]
    fn numeric_lowering_resolves_exact_midpoints_without_double_rounding() -> Result<()> {
        let tiny=q("1/10000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000");
        for low in [0.,f32::MIN_POSITIVE,0.1,1.,-1.,1e20] {
            let high=low.next_up();
            let midpoint=(ieee(low)?+ieee(high)?)/q("2");
            assert_eq!(Germ::point(&midpoint-&tiny).numeric()?,low);
            assert_eq!(Germ::point(&midpoint+&tiny).numeric()?,high);
            assert_eq!(Germ::point(midpoint.clone()).numeric()?,if low.to_bits()&1==0 {low}else{high});
            assert_eq!(Germ::left(midpoint.clone()).numeric_side()?,low);
            assert_eq!(Germ::right(midpoint).numeric_side()?,high);
        }
        Ok(())
    }
    #[test]
    fn authored_decimals_and_ieee_buffers_have_distinct_meanings() {
        assert_ne!(q("0.1"), ieee(0.1).unwrap());
        assert_eq!(q("29.97"), q("2997/100"));
        assert_eq!(q("-1.25e-2"), q("-1/80"));
        assert!(ieee(1e-20).unwrap().denom() > &BigInt::from(i64::MAX));
    }
    #[test]
    fn reverse_loop_and_history_preserve_the_approached_branch() {
        let left = Germ::left(q("1/2"));
        let reverse = |t: &Germ| Germ::point(q("1")).sub(t);
        let right = reverse(&left);
        assert_eq!(right, Germ::right(q("1/2")));
        assert_eq!(reverse(&right), left);
        assert_eq!(right.sub(&Germ::point(q("1/24"))), Germ::right(q("11/24")));
        assert_eq!(Germ::left(q("1")).euclidean_loop(&q("0"), &q("1")).unwrap(), Germ::left(q("1")));
        assert_eq!(Germ::point(q("1")).euclidean_loop(&q("0"), &q("1")).unwrap(), Germ::point(q("0")));
        assert_eq!(Germ::right(q("1")).euclidean_loop(&q("0"), &q("1")).unwrap(), Germ::right(q("0")));
    }
    #[test]
    fn extrema_and_blend_cancellation_keep_higher_terms() {
        let epsilon = Germ::left(q("0"));
        let square = epsilon.mul(&epsilon);
        assert_eq!(square.compare_point(&q("0")), Ordering::Greater);
        assert_eq!(square.scale(&q("-1")).compare_point(&q("0")), Ordering::Less);
        let a = Germ::left(q("1/2"));
        let b = a.sub(&square);
        assert_eq!(a.compare_point(&q("1/2")), b.compare_point(&q("1/2")));
        assert_eq!(a.sub(&b).compare_point(&q("0")), Ordering::Greater);
        assert_eq!(a.sub(&a).compare_point(&q("0")), Ordering::Equal);
    }
    #[test]
    fn derived_values_have_no_fixed_width_bound() {
        let mut value = Germ::point(q("0.1234567"));
        for _ in 0..8 { value = value.mul(&value).mul(&value); }
        assert!(value.value().denom().bits() > 100_000);
        assert_eq!(Germ::point(q("1e100")).floor_clamped(0, 30).unwrap(), 30);
        assert_eq!(Germ::left(q("0")).floor_clamped(-10, 10).unwrap(), -1);
    }
}

impl Germ {
    /// IEEE evaluation on the approached side, for discrete decisions driven by
    /// numeric controls. Ordinary continuous evaluation still uses `numeric`.
    pub fn numeric_side(&self) -> Result<f32> {
        let value=self.numeric()?;
        for (neighbor,order) in [(value.next_down(),Ordering::Less),(value.next_up(),Ordering::Greater)] {
            if neighbor.is_finite() {
                let midpoint=(ieee(value)?+ieee(neighbor)?)/Rational::from_integer(2.into());
                if self.value()==&midpoint && self.compare_point(&midpoint)==order {return Ok(neighbor)}
            }
        }
        Ok(value)
    }
}
