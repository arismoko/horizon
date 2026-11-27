//! Java's floating-point spelling, shared by SNBT, JSON and player-facing text.
//!
//! Decimal selection follows the public Java 25 `Double.toString` and
//! `Float.toString` contracts. No Java runtime implementation source is used.

use std::fmt::{self, Write};

mod shortest;

// Scientific f64 text (including sign) needs at most 24 ASCII bytes.
struct NumberBuffer {
    bytes: [u8; 24],
    len: usize,
}

impl Write for NumberBuffer {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let end = self.len + text.len();
        let destination = self.bytes.get_mut(self.len..end).ok_or(fmt::Error)?;
        destination.copy_from_slice(text.as_bytes());
        self.len = end;
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct Decimal {
    digits: [u8; 24],
    len: usize,
    // Position of the decimal point relative to the first significant digit.
    point: i32,
}

impl Decimal {
    fn from_shortest(value: shortest::Digits) -> Self {
        let ascii = value.bytes();
        let mut digits = [0; 24];
        digits[..ascii.len()].copy_from_slice(ascii);
        Self {
            digits,
            len: ascii.len(),
            point: value.exponent + 1,
        }
    }

    fn from_text(text: &str) -> Self {
        let (mantissa, exponent) = text.split_once('e').unwrap_or((text, "0"));
        let mut digits = [0; 24];
        let mut len = 0;
        let mut point = mantissa.len() as i32;
        for b in mantissa.bytes() {
            if b == b'.' {
                point = len as i32;
            } else {
                digits[len] = b;
                len += 1;
            }
        }
        point += exponent.parse::<i32>().expect("decimal exponent");
        if let Some(leading) = digits[..len].iter().position(|&b| b != b'0') {
            digits.copy_within(leading..len, 0);
            point -= leading as i32;
            len -= leading;
            while len > 1 && digits[len - 1] == b'0' {
                len -= 1;
            }
            Self { digits, len, point }
        } else {
            Self::zero()
        }
    }

    fn zero() -> Self {
        let mut digits = [0; 24];
        digits[0] = b'0';
        Self {
            digits,
            len: 1,
            point: 1,
        }
    }

    fn with_two_digit_selection(self, value: f64, tiny: bool) -> Self {
        // Java considers two significant digits when one is enough to round-trip.
        // Only the smallest subnormals have a rounding interval wide enough to
        // include a different two-digit decimal: even the narrow side of that
        // decimal grid is >= 0.5% of the one-digit value, but after bit 128 the
        // half-ulp is < 0.4% of the value.
        if self.len != 1 || !tiny || value == 0.0 {
            return self;
        }
        let mut nearest = NumberBuffer {
            bytes: [0; 24],
            len: 0,
        };
        write!(nearest, "{value:.1e}").expect("two-digit scientific text fits");
        Self::from_text(std::str::from_utf8(&nearest.bytes[..nearest.len]).expect("ASCII digits"))
    }

    fn as_str(&self) -> &str {
        std::str::from_utf8(&self.digits[..self.len]).expect("ASCII digits")
    }

    fn fixed_string(self, negative: bool, decimals: usize) -> String {
        let width = self.point.max(1) as usize + usize::from(negative);
        let mut out = String::with_capacity(width + decimals + usize::from(decimals > 0));
        if negative {
            out.push('-');
        }
        self.push_fixed(&mut out, decimals);
        out
    }

    fn push_fixed(&self, out: &mut String, decimals: usize) {
        let digits = self.as_str();
        let split = (self.point.max(0) as usize).min(digits.len());
        if self.point <= 0 {
            out.push('0');
        } else {
            out.push_str(&digits[..split]);
            out.extend(std::iter::repeat_n('0', self.point as usize - split));
        }
        if decimals == 0 {
            return;
        }
        out.push('.');
        let leading = ((-self.point).max(0) as usize).min(decimals);
        let fractional = &digits[split..split + (digits.len() - split).min(decimals - leading)];
        out.extend(std::iter::repeat_n('0', leading));
        out.push_str(fractional);
        out.extend(std::iter::repeat_n(
            '0',
            decimals - leading - fractional.len(),
        ));
    }

    fn round_half_up(mut self, decimals: usize) -> Self {
        // No finite double has a fractional significant digit after position 340.
        let keep = self.point + decimals.min(340) as i32;
        if keep < 0 {
            return Self::zero();
        }
        let keep = keep as usize;
        if keep >= self.len {
            return self;
        }
        let up = self.digits[keep] >= b'5';
        self.len = keep;
        if up {
            for i in (0..keep).rev() {
                if self.digits[i] != b'9' {
                    self.digits[i] += 1;
                    return self;
                }
                self.digits[i] = b'0';
            }
            self.digits[0] = b'1';
            self.len = 1;
            self.point += 1;
        } else if keep == 0 {
            return Self::zero();
        }
        self
    }
}

#[inline]
fn double_digits(magnitude: f64) -> Decimal {
    Decimal::from_shortest(shortest::double(magnitude).digits::<false>())
        .with_two_digit_selection(magnitude, magnitude.to_bits() <= 128)
}

fn special(value: f64) -> Option<&'static str> {
    (!value.is_finite()).then(|| nonfinite_text(value.is_sign_negative(), value.is_nan()))
}

fn nonfinite_text(negative: bool, nan: bool) -> &'static str {
    if nan {
        "NaN"
    } else if negative {
        "-Infinity"
    } else {
        "Infinity"
    }
}

// Copy the first and last fixed-size chunks to avoid a call to memcpy on
// short mantissas. When the chunks overlap they write the same digits.
#[inline(always)]
fn copy_digits(dst: &mut [u8], digits: &[u8]) {
    match digits.len() {
        0 => {}
        1 => dst[0] = digits[0],
        n @ 2..=3 => {
            dst[..2].copy_from_slice(&digits[..2]);
            dst[n - 2..].copy_from_slice(&digits[n - 2..]);
        }
        n @ 4..=7 => {
            dst[..4].copy_from_slice(&digits[..4]);
            dst[n - 4..].copy_from_slice(&digits[n - 4..]);
        }
        n @ 8..=16 => {
            dst[..8].copy_from_slice(&digits[..8]);
            dst[n - 8..].copy_from_slice(&digits[n - 8..]);
        }
        _ => dst.copy_from_slice(digits),
    }
}

#[inline(always)]
fn java_string<const SIZE: usize>(negative: bool, digits: &[u8], exponent: i32) -> String {
    // At most 24 bytes for binary64 (17 digits, sign, point, E, signed
    // three-digit exponent) and 16 for binary32 (at most 9 digits and a
    // signed two-digit exponent). The unused bytes stay ASCII as well.
    let mut out = [b'0'; SIZE];
    let start = usize::from(negative);
    if negative {
        out[0] = b'-';
    }
    let end = if (0..7).contains(&exponent) {
        let point = exponent as usize + 1;
        let split = point.min(digits.len());
        copy_digits(&mut out[start..start + split], &digits[..split]);
        let point = start + point;
        out[point] = b'.';
        let fraction = &digits[split..];
        let end = point + 1 + fraction.len();
        copy_digits(&mut out[point + 1..end], fraction);
        end + usize::from(fraction.is_empty())
    } else if (-3..0).contains(&exponent) {
        out[start + 1] = b'.';
        let first_digit = start + 1 + exponent.unsigned_abs() as usize;
        let end = first_digit + digits.len();
        copy_digits(&mut out[first_digit..end], digits);
        end
    } else {
        out[start] = digits[0];
        out[start + 1] = b'.';
        let mantissa_end = start + 1 + digits.len();
        copy_digits(&mut out[start + 2..mantissa_end], &digits[1..]);
        let mut pos = mantissa_end + usize::from(digits.len() == 1);
        out[pos] = b'E';
        pos += 1;
        if exponent < 0 {
            out[pos] = b'-';
            pos += 1;
        }
        let n = exponent.unsigned_abs();
        if n >= 100 {
            out[pos] = b'0' + (n / 100) as u8;
            pos += 1;
        }
        if n >= 10 {
            out[pos] = b'0' + (n / 10 % 10) as u8;
            pos += 1;
        }
        out[pos] = b'0' + (n % 10) as u8;
        pos + 1
    };
    // Binary32's full buffer fits one ASCII validation block on 64-bit
    // targets. For binary64, skip validating the unused part of the buffer.
    let validate = if SIZE == 16 { &out[..] } else { &out[..end] };
    std::str::from_utf8(validate).expect("ASCII floating-point text")[..end].into()
}

// Keep the exceptional two-digit subnormal conversion out of the hot
// formatter. It needs the standard formatter only for a handful of raw bits.
#[cold]
#[inline(never)]
fn java_tiny_string<const SIZE: usize>(
    negative: bool,
    value: f64,
    digits: shortest::Digits,
) -> String {
    let decimal = Decimal::from_shortest(digits).with_two_digit_selection(value, true);
    java_string::<SIZE>(negative, &decimal.digits[..decimal.len], decimal.point - 1)
}

/// `Double.toString`.
pub fn double_to_string(value: f64) -> String {
    if let Some(text) = special(value) {
        return text.into();
    }
    let magnitude = value.abs();
    let digits = shortest::double(magnitude).digits::<false>();
    if magnitude.to_bits() <= 128 && magnitude != 0.0 && digits.is_single() {
        return java_tiny_string::<24>(value.is_sign_negative(), magnitude, digits);
    }
    java_string::<24>(value.is_sign_negative(), digits.bytes(), digits.exponent)
}

/// `Float.toString`.
pub fn float_to_string(value: f32) -> String {
    let magnitude = value.abs();
    if !magnitude.is_finite() {
        return nonfinite_text(value.is_sign_negative(), magnitude.is_nan()).into();
    }
    let digits = shortest::float(magnitude).digits::<true>();
    if magnitude.to_bits() <= 128 && magnitude != 0.0 && digits.is_single() {
        return java_tiny_string::<16>(value.is_sign_negative(), f64::from(magnitude), digits);
    }
    java_string::<16>(value.is_sign_negative(), digits.bytes(), digits.exponent)
}

/// `String.format(Locale.ROOT, "%.<precision>f", value)`: Java rounds the
/// chosen shortest decimal expansion half-up (so 1.005 gives 1.01).
pub fn format_fixed(value: f64, decimals: usize) -> String {
    if let Some(text) = special(value) {
        return text.into();
    }
    double_digits(value.abs())
        .round_half_up(decimals)
        .fixed_string(value.is_sign_negative(), decimals)
}

/// `%.2f`.
pub fn format_fixed2(value: f64) -> String {
    format_fixed(value, 2)
}

/// US/ROOT `DecimalFormat` patterns used by the game, without grouping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecimalPattern {
    /// `#.##`, with DecimalFormat's implicit minimum of one integer digit.
    Tooltip,
    /// `########0.00`.
    Statistic,
    /// `0.0###`.
    Integrity,
    /// `#` after setting the maximum fraction digits to 15, no leading zero.
    Macro,
}

/// US/ROOT `DecimalFormat` with two optional or mandatory fractional digits.
pub fn format_decimal2(value: f64, trim: bool) -> String {
    let pattern = if trim {
        DecimalPattern::Tooltip
    } else {
        DecimalPattern::Statistic
    };
    format_decimal(value, pattern)
}

/// Locale-independent game patterns. Round half-even on the binary value
/// when shortening the selected digits, not half-up as `String.format` does.
pub fn format_decimal(value: f64, pattern: DecimalPattern) -> String {
    if value.is_infinite() {
        return if value.is_sign_negative() {
            "-∞"
        } else {
            "∞"
        }
        .into();
    }
    if value.is_nan() {
        return "NaN".into();
    }
    // Below these binary exponents the rounding interval is less than half
    // of a decimal unit at the pattern's maximum precision.
    let (min_fraction, max_fraction, check_digits_at) = match pattern {
        DecimalPattern::Tooltip => (0, 2, 45),
        DecimalPattern::Statistic => (2, 2, 45),
        DecimalPattern::Integrity => (1, 4, 38),
        DecimalPattern::Macro => (0, 15, 2),
    };
    let magnitude = value.abs();
    let binary_exponent = ((magnitude.to_bits() >> 52) as i32) - 1023;
    if binary_exponent >= 53 {
        // DecimalFormat's integral conversion is different from Double.toString.
        let integer = if binary_exponent < 63 {
            let scale = 10u64.pow((1u64 << (binary_exponent - 54).max(0)).ilog10());
            ((magnitude as u64 + scale / 2) / scale * scale).to_string()
        } else {
            decimal_format_large_integer(magnitude, binary_exponent)
        };
        if min_fraction == 0 && !value.is_sign_negative() {
            return integer;
        }
        let mut out = String::with_capacity(integer.len() + min_fraction + 2);
        if value.is_sign_negative() {
            out.push('-');
        }
        out.push_str(&integer);
        if min_fraction != 0 {
            out.push('.');
            out.extend(std::iter::repeat_n('0', min_fraction));
        }
        return out;
    }
    let mut out = if binary_exponent >= check_digits_at {
        let digits = double_digits(magnitude);
        if digits.point + max_fraction as i32 >= digits.len as i32 {
            digits.fixed_string(value.is_sign_negative(), max_fraction)
        } else {
            format!("{value:.max_fraction$}")
        }
    } else {
        format!("{value:.max_fraction$}")
    };
    if min_fraction < max_fraction {
        let fraction_start = out.len() - max_fraction;
        let trimmed = out
            .trim_end_matches('0')
            .len()
            .max(fraction_start + min_fraction);
        out.truncate(if trimmed == fraction_start {
            fraction_start - 1
        } else {
            trimmed
        });
    }
    if pattern == DecimalPattern::Macro {
        let start = usize::from(value.is_sign_negative());
        if out.as_bytes().get(start..start + 2) == Some(b"0.") {
            out.remove(start);
        }
    }
    out
}

/// DecimalFormat's long-exceeding integral doubles use the closest decimal in
/// the *open* interval of half a spacing, using the smaller adjacent spacing
/// at a power of two. Unlike Double.toString, an endpoint that parses back to
/// the input is not enough (e.g. 1e23). Compare exact decimal integer distances.
fn decimal_format_large_integer(value: f64, binary_exponent: i32) -> String {
    let exact = format!("{value:.0}");
    let bytes = exact.as_bytes();
    let power_of_two = value.to_bits() & ((1 << 52) - 1) == 0;
    let half =
        f64::from_bits(((binary_exponent - 53 - i32::from(power_of_two) + 1023) as u64) << 52);
    let limit = format!("{half:.0}");
    let limit = limit.as_bytes();
    let mut distance = [0; 309];
    // In the legacy signed-integer digit loop, the numerator and margin are
    // scaled by ten for each emitted digit. Where that loop's operands fit a
    // long, overflow of their sum can suppress the upper-bound test even
    // though a lower candidate is close enough. E.g. 0x453d9940fff0aafa.
    let denominator = (bytes.len() <= 26)
        .then(|| {
            let decimal_exponent = bytes.len() as i32 - 1;
            let binary_margin = binary_exponent - 53 - i32::from(power_of_two);
            5u64.pow(decimal_exponent as u32)
                .checked_mul(1u64 << (decimal_exponent - binary_margin).max(0))
                .filter(|&d| d <= i64::MAX as u64 / 10)
        })
        .flatten();
    for keep in 1..bytes.len() {
        let tail = &bytes[keep..];
        let nonzero = tail.iter().rposition(|&c| c != b'0');
        let mut up = tail[0] > b'5'
            || (tail[0] == b'5' && (nonzero.is_some_and(|i| i > 0) || bytes[keep - 1] & 1 != 0));
        let delta = if up {
            // 10^n - tail, expressed by complement and increment without overflow.
            let last = nonzero.expect("rounded up on a nonzero tail");
            for (dst, &digit) in distance[..last].iter_mut().zip(tail) {
                *dst = b'9' - (digit - b'0');
            }
            distance[last] = b'0' + (b':' - tail[last]);
            distance[last + 1..tail.len()].fill(b'0');
            &distance[..tail.len()]
        } else {
            tail
        };
        let delta = &delta[delta.iter().position(|&c| c != b'0').unwrap_or(delta.len())..];
        if (delta.len(), delta) >= (limit.len(), limit) {
            continue;
        }
        if up
            && let Some(denominator) = denominator
            && tail.len() <= 18
        {
            let remainder: u128 = std::str::from_utf8(tail)
                .expect("ASCII digits")
                .parse()
                .expect("18 digits fit");
            let scale = u128::from(denominator) * 10;
            let divisor = 10u128.pow(tail.len() as u32);
            let numerator = remainder * scale / divisor;
            let margin = u128::from(half as u64) * scale / divisor;
            if margin <= i64::MAX as u128 && numerator + margin > i64::MAX as u128 {
                if remainder >= u128::from(half as u64) {
                    continue;
                }
                up = false;
            }
        }
        let mut rounded = [b'0'; 310];
        rounded[1..=keep].copy_from_slice(&bytes[..keep]);
        if up {
            for digit in rounded[..=keep].iter_mut().rev() {
                if *digit < b'9' {
                    *digit += 1;
                    break;
                }
                *digit = b'0';
            }
        }
        let start = usize::from(rounded[0] == b'0');
        return std::str::from_utf8(&rounded[start..bytes.len() + 1])
            .expect("ASCII digits")
            .into();
    }
    exact
}

#[cfg(test)]
mod moved_tests {
    use super::*;

    #[test]
    fn java_formatter_rounds_selected_decimal_digits_half_up() {
        // Temurin 25 Locale.ROOT: shortest selection is nearest-even; fixed
        // precision then rounds the selected decimal half-up.
        let midpoint = f64::from_bits(0x430c_6bf5_2634_0002);
        assert_eq!(format_fixed(midpoint, 3), "1000000000000000.200");
        assert_eq!(format_fixed2(2.675), "2.68");
        assert_eq!(format_fixed(0.5, 0), "1");
        assert_eq!(format_fixed2(-0.001), "-0.00");
        assert_eq!(format_fixed2(f64::NEG_INFINITY), "-Infinity");
        let tiny = format!("0.{}99{}", "0".repeat(323), "0".repeat(15));
        assert_eq!(format_fixed(f64::from_bits(2), 340), tiny);
    }

    #[test]
    fn java_decimal_format_rounds_binary_ties_and_uses_legacy_large_digits() {
        // Temurin 25 DecimalFormat with Locale.US symbols, not String.format.
        assert_eq!(format_decimal2(2.675, true), "2.67");
        assert_eq!(format_decimal2(0.125, false), "0.12");
        assert_eq!(format_decimal2(-0.0, true), "-0");
        assert_eq!(format_decimal2(f64::INFINITY, false), "∞");
        assert_eq!(format_decimal2(1e23, true), "99999999999999990000000");
        assert_eq!(
            format_decimal2(51621604283940224.0, false),
            "51621604283940224.00"
        );
        let upper_overflow = f64::from_bits(0x453d_9940_fff0_aafa);
        assert_eq!(
            format_decimal2(upper_overflow, true),
            "35782569874752483000000000"
        );
    }
    #[test]
    fn java_number_text() {
        assert_eq!(double_to_string(1.0), "1.0");
        assert_eq!(double_to_string(0.5), "0.5");
        assert_eq!(double_to_string(0.0001), "1.0E-4");
        assert_eq!(double_to_string(0.001), "0.001");
        assert_eq!(double_to_string(1234567.0), "1234567.0");
        assert_eq!(double_to_string(12345678.0), "1.2345678E7");
        assert_eq!(double_to_string(100.0), "100.0");
        assert_eq!(float_to_string(0.15), "0.15");
        assert_eq!(float_to_string(1.0e10), "1.0E10");
        assert_eq!(double_to_string(-2.5), "-2.5");
        for (bits, expected) in [
            (0x0000_0001, "1.4E-45"),
            (0x0000_0002, "2.8E-45"),
            (0x0000_0007, "9.8E-45"),
            (0x007f_ffff, "1.1754942E-38"),
            (0x0080_0000, "1.1754944E-38"),
            (0x0080_0001, "1.1754945E-38"),
            (0x3980_0000, "2.4414062E-4"),
            (0x44a4_5900, "1314.7812"),
            (0x4811_6728, "148892.62"),
            (0x4a00_0001, "2097152.2"),
            (0x10fd_8a53, "1.00004027E-28"),
            (0x7f7f_ffff, "3.4028235E38"),
        ] {
            assert_eq!(float_to_string(f32::from_bits(bits)), expected);
            assert_eq!(
                float_to_string(f32::from_bits(bits | 0x8000_0000)),
                format!("-{expected}")
            );
        }
        for (bits, expected) in [
            (0x0000_0000_0000_0001, "4.9E-324"),
            (0x0000_0000_0000_0002, "9.9E-324"),
            (0x0000_0000_0000_0014, "9.9E-323"),
            (0x000f_ffff_ffff_ffff, "2.225073858507201E-308"),
            (0x0010_0000_0000_0000, "2.2250738585072014E-308"),
            (0x0010_0000_0000_0001, "2.225073858507202E-308"),
            (0x3fef_ffff_ffff_ffff, "0.9999999999999999"),
            (0x3ff0_0000_0000_0001, "1.0000000000000002"),
            (0x3e60_0000_0000_0000, "2.9802322387695312E-8"),
            (0x4300_0ef8_712f_e8d2, "5.650074801267462E14"),
            (0x4310_0000_0000_0001, "1.1258999068426242E15"),
            (0x7fef_ffff_ffff_ffff, "1.7976931348623157E308"),
        ] {
            assert_eq!(double_to_string(f64::from_bits(bits)), expected);
            assert_eq!(
                double_to_string(f64::from_bits(bits | 0x8000_0000_0000_0000)),
                format!("-{expected}")
            );
        }
        assert_eq!(float_to_string(-0.0), "-0.0");
        assert_eq!(double_to_string(-0.0), "-0.0");
        assert_eq!(float_to_string(f32::INFINITY), "Infinity");
        assert_eq!(double_to_string(f64::NEG_INFINITY), "-Infinity");
        assert_eq!(float_to_string(f32::NAN), "NaN");
        assert_eq!(double_to_string(f64::NAN), "NaN");
    }
}
