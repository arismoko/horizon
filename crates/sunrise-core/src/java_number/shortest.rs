//! Schubfach digit selection, adapted from David Tolnay's MIT-licensed zmij 1.0.23
//! (https://github.com/dtolnay/zmij/tree/7b7cc48b58028e8af7be87e94c0c1c8936f1a57c),
//! a Rust port of Victor Zverovich's implementation. This module selects digits;
//! Java's notation and exceptional values are handled by its caller.

// Permission is hereby granted, free of charge, to any
// person obtaining a copy of this software and associated
// documentation files (the "Software"), to deal in the
// Software without restriction, including without
// limitation the rights to use, copy, modify, merge,
// publish, distribute, sublicense, and/or sell copies of
// the Software, and to permit persons to whom the Software
// is furnished to do so, subject to the following
// conditions:
//
// The above copyright notice and this permission notice
// shall be included in all copies or substantial portions
// of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF
// ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED
// TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A
// PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT
// SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY
// CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION
// OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR
// IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
// DEALINGS IN THE SOFTWARE.

#[derive(Clone, Copy)]
struct Power {
    hi: u64,
    lo: u64,
}

const fn high_product(a: u64, b: u64) -> u64 {
    ((a as u128 * b as u128) >> 64) as u64
}

#[rustfmt::skip]
const POW10_MINOR: [u64; 28] = [
    0x8000000000000000, 0xa000000000000000, 0xc800000000000000,
    0xfa00000000000000, 0x9c40000000000000, 0xc350000000000000,
    0xf424000000000000, 0x9896800000000000, 0xbebc200000000000,
    0xee6b280000000000, 0x9502f90000000000, 0xba43b74000000000,
    0xe8d4a51000000000, 0x9184e72a00000000, 0xb5e620f480000000,
    0xe35fa931a0000000, 0x8e1bc9bf04000000, 0xb1a2bc2ec5000000,
    0xde0b6b3a76400000, 0x8ac7230489e80000, 0xad78ebc5ac620000,
    0xd8d726b7177a8000, 0x878678326eac9000, 0xa968163f0a57b400,
    0xd3c21bcecceda100, 0x84595161401484a0, 0xa56fa5b99019a5c8,
    0xcecb8f27f4200f3a,
];

#[rustfmt::skip]
const POW10_MAJOR: [Power; 23] = [
    Power { hi: 0xaf8e5410288e1b6f, lo: 0x07ecf0ae5ee44dda }, // -303
    Power { hi: 0xb1442798f49ffb4a, lo: 0x99cd11cfdf41779d }, // -275
    Power { hi: 0xb2fe3f0b8599ef07, lo: 0x861fa7e6dcb4aa15 }, // -247
    Power { hi: 0xb4bca50b065abe63, lo: 0x0fed077a756b53aa }, // -219
    Power { hi: 0xb67f6455292cbf08, lo: 0x1a3bc84c17b1d543 }, // -191
    Power { hi: 0xb84687c269ef3bfb, lo: 0x3d5d514f40eea742 }, // -163
    Power { hi: 0xba121a4650e4ddeb, lo: 0x92f34d62616ce413 }, // -135
    Power { hi: 0xbbe226efb628afea, lo: 0x890489f70a55368c }, // -107
    Power { hi: 0xbdb6b8e905cb600f, lo: 0x5400e987bbc1c921 }, //  -79
    Power { hi: 0xbf8fdb78849a5f96, lo: 0xde98520472bdd034 }, //  -51
    Power { hi: 0xc16d9a0095928a27, lo: 0x75b7053c0f178294 }, //  -23
    Power { hi: 0xc350000000000000, lo: 0x0000000000000000 }, //    5
    Power { hi: 0xc5371912364ce305, lo: 0x6c28000000000000 }, //   33
    Power { hi: 0xc722f0ef9d80aad6, lo: 0x424d3ad2b7b97ef6 }, //   61
    Power { hi: 0xc913936dd571c84c, lo: 0x03bc3a19cd1e38ea }, //   89
    Power { hi: 0xcb090c8001ab551c, lo: 0x5cadf5bfd3072cc6 }, //  117
    Power { hi: 0xcd036837130890a1, lo: 0x36dba887c37a8c10 }, //  145
    Power { hi: 0xcf02b2c21207ef2e, lo: 0x94f967e45e03f4bc }, //  173
    Power { hi: 0xd106f86e69d785c7, lo: 0xe13336d701beba52 }, //  201
    Power { hi: 0xd31045a8341ca07c, lo: 0x1ede48111209a051 }, //  229
    Power { hi: 0xd51ea6fa85785631, lo: 0x552a74227f3ea566 }, //  257
    Power { hi: 0xd732290fbacaf133, lo: 0xa97c177947ad4096 }, //  285
    Power { hi: 0xd94ad8b1c7380874, lo: 0x18375281ae7822bc }, //  313
];

#[rustfmt::skip]
const POW10_FIXUPS: [u32; 20] = [
    0x0a4e363f, 0x00001840, 0x00006400, 0x24200040, 0x00000000,
    0x0c000000, 0x82c81380, 0x5e4ce01f, 0xd730f60f, 0x0000001b,
    0x00000000, 0xcdf7fffc, 0x6e8201d8, 0x40cd3fd1, 0xdb642501,
    0x00000d0d, 0x14042400, 0x53713840, 0x11781db4, 0x00000000,
];

// Each entry is the normalized 128-bit significand of 10^(i - 293),
// rounded down. The major/minor tables and one-bit fixups build the 9.7 KiB
// cache at compile time; no generated source or build script is needed.
const POW10: [Power; 618] = {
    let mut result = [Power { hi: 0, lo: 0 }; 618];
    let mut i = 0;
    while i < result.len() {
        let m = POW10_MINOR[(i + 10) % 28];
        let h = POW10_MAJOR[(i + 10) / 28];
        let low = h.lo.wrapping_mul(m);
        let carry = high_product(h.lo, m);
        let (middle, overflow) = carry.overflowing_add(h.hi.wrapping_mul(m));
        let high = high_product(h.hi, m) + overflow as u64;
        let fixup = ((POW10_FIXUPS[i >> 5] >> (i & 31)) & 1) as u64;
        result[i] = if high >> 63 != 0 {
            Power {
                hi: high,
                lo: middle - fixup,
            }
        } else {
            Power {
                hi: (high << 1) | (middle >> 63),
                lo: ((middle << 1) | (low >> 63)) - fixup,
            }
        };
        i += 1;
    }
    result
};

/// Unsigned numeric digits with the exponent of the least significant digit.
#[derive(Clone, Copy)]
pub(super) struct Shortest {
    significand: u64,
    exponent: i32,
}

pub(super) struct Digits {
    bytes: [u8; 17],
    start: usize,
    end: usize,
    pub(super) exponent: i32,
}

impl Digits {
    pub(super) fn is_single(&self) -> bool {
        self.end == self.start + 1
    }

    #[inline]
    pub(super) fn bytes(&self) -> &[u8] {
        &self.bytes[self.start..self.end]
    }
}

/// Convert eight padded base-10 digits into eight bytes, in parallel. First
/// split into groups of four, then pairs, then single digits. Each product
/// is modulo 2^64: only its indicated lane bits are used.
#[inline]
fn bcd8(value: u32) -> u64 {
    let four = u64::from(value) + ((1 << 32) - 10_000) * u64::from(value / 10_000);
    let two = four + ((1 << 16) - 100) * ((four.wrapping_mul(5243) >> 19) & 0x7f_0000_007f);
    two + ((1 << 8) - 10) * ((two.wrapping_mul(103) >> 10) & 0x000f_000f_000f_000f)
}

impl Shortest {
    #[inline]
    pub(super) fn digits<const FLOAT: bool>(self) -> Digits {
        // Keep one extra place at the front for binary64's 17th digit or
        // binary32's ninth. Before adding ASCII '0', the leading/trailing
        // all-zero bytes can be counted away with integer bit operations.
        const ASCII_ZERO: u64 = u64::from_be_bytes([b'0'; 8]);
        let mut bytes = [0; 17];
        let (start, end) = if FLOAT {
            let value = self.significand as u32;
            // Most binary32 significands fit in one BCD word.
            let (high, low) = if value < 100_000_000 {
                (0, bcd8(value))
            } else {
                (value / 100_000_000, bcd8(value % 100_000_000))
            };
            bytes[8] = b'0' + high as u8;
            bytes[9..].copy_from_slice(&(low | ASCII_ZERO).to_be_bytes());
            let start = if high != 0 {
                8
            } else {
                9 + (low.leading_zeros() / 8).min(7) as usize
            };
            let end = 17 - (low.trailing_zeros() / 8) as usize;
            (start, end.max(start + 1))
        } else {
            let low = bcd8((self.significand % 100_000_000) as u32);
            let head = self.significand / 100_000_000;
            let top = head / 100_000_000;
            let middle = bcd8((head % 100_000_000) as u32);
            bytes[0] = b'0' + top as u8;
            bytes[1..9].copy_from_slice(&(middle | ASCII_ZERO).to_be_bytes());
            bytes[9..].copy_from_slice(&(low | ASCII_ZERO).to_be_bytes());
            let start = if top != 0 {
                0
            } else if middle != 0 {
                1 + (middle.leading_zeros() / 8) as usize
            } else {
                9 + (low.leading_zeros() / 8).min(7) as usize
            };
            let end = if low != 0 {
                17 - (low.trailing_zeros() / 8) as usize
            } else {
                9 - (middle.trailing_zeros() / 8) as usize
            };
            (start, end.max(start + 1))
        };
        // The all-zero value has no significance or decimal scale.
        let exponent = if self.significand == 0 {
            0
        } else {
            self.exponent + (16 - start) as i32
        };
        Digits {
            bytes,
            start,
            end,
            exponent,
        }
    }
}

#[inline]
pub(super) fn double(value: f64) -> Shortest {
    let bits = value.to_bits();
    select::<52, 1075>(bits & ((1 << 52) - 1), ((bits >> 52) & 0x7ff) as i32)
}

// The 4 KiB binary32 interval cache needs only the high power limb.
// The half-ulp uses at most 35 bits, leaving space for its shift and
// signed decimal exponent in the same word.
const FLOAT_SCALE: [[u64; 2]; 256] = {
    let mut table = [[0; 2]; 256];
    let mut raw_exp = 0;
    while raw_exp < 255 {
        let q = if raw_exp == 0 { 1 } else { raw_exp as i32 } - 150;
        let (k, scale) = decimal_scale(q, false);
        let shift = scale + 34;
        let power = POW10[(292 - k) as usize].hi;
        let half = power >> (65 - shift);
        table[raw_exp] = [
            power + 1,
            half | (shift as u64) << 48 | (k as u8 as u64) << 56,
        ];
        raw_exp += 1;
    }
    table
};

#[inline]
pub(super) fn float(value: f32) -> Shortest {
    let bits = value.to_bits();
    let exponent = ((bits >> 23) & 0xff) as i32;
    let fraction = bits & ((1 << 23) - 1);
    if fraction == 0 {
        return select::<23, 150>(0, exponent);
    }
    // An exact integer below 2^25 needs no interval search: the half-ulp is at
    // most one, and above 2^24 both the value and any multiple
    // of ten are even, so a distinct multiple can never be that close.
    if (127..=151).contains(&exponent) {
        let c = fraction | (1 << 23);
        let fraction_bits = (150 - exponent).max(0) as u32;
        if c & ((1 << fraction_bits) - 1) == 0 {
            return Shortest {
                significand: u64::from((c >> fraction_bits) << u32::from(exponent == 151)),
                exponent: 0,
            };
        }
    }
    let [power, scale] = FLOAT_SCALE[exponent as usize];
    let c = u64::from(fraction | ((exponent != 0) as u32) << 23);
    let p = high_product(power, c << ((scale >> 48) as u8));
    let integral = p >> 34;
    let rem = p & ((1 << 34) - 1);
    let half = (scale & ((1 << 48) - 1)) + 1 - (c & 1);
    let up = (rem + half) >> 34 != 0;
    let k = i32::from((scale >> 56) as i8);
    if up || half > rem {
        Shortest {
            significand: integral + u64::from(up),
            exponent: k + 1,
        }
    } else {
        let digit = if rem == 1 << 32 {
            2
        } else {
            (rem * 10 + (1 << 33)) >> 34
        };
        Shortest {
            significand: integral * 10 + digit,
            exponent: k,
        }
    }
}

// floor(log10(2^q)), or floor(log10(3/4 * 2^q)) for the shorter interval,
// and the shift to align the result of multiplying by 10^(-k-1).
const fn decimal_scale(q: i32, irregular: bool) -> (i32, i32) {
    let k = (q * 315_653 - (irregular as i32) * 131_072) >> 20;
    (k, q + ((-(k + 1) * 217_707) >> 16) + 1)
}

#[inline]
fn select<const MANTISSA: u32, const BIAS: i32>(fraction: u64, raw_exp: i32) -> Shortest {
    if raw_exp == 0 && fraction == 0 {
        return Shortest {
            significand: 0,
            exponent: 0,
        };
    }
    let c = fraction | ((raw_exp != 0) as u64) << MANTISSA;
    let q = raw_exp.max(1) - BIAS;
    // Powers of two have a half-width lower rounding interval, except at
    // the normal/subnormal boundary. Keep the zmij power-of-two selection
    // at that boundary, where both intervals choose the same decimal.
    let irregular = fraction == 0;
    let (k, scale) = decimal_scale(q, irregular);
    // Scale both the centre and rounding interval by 10^(-k-1). Compute
    // the shorter integer directly; only add a digit if it does not fit.
    let power = POW10[(292 - k) as usize];
    const BITS: i32 = 6;
    let shift = (scale + BITS) as u32;
    let p = u128::from(power.hi) * u128::from(c << shift)
        + u128::from(high_product(power.lo, c << shift));
    let integral = (p >> (64 + BITS)) as u64;
    let rem = (p >> BITS) as u64;
    let half = (power.hi >> (BITS as u32 + 1 - shift)) + u64::from(!irregular && c & 1 == 0);
    let up = half > u64::MAX - rem;
    let round = |x, bias| ((u128::from(x) * 10 + u128::from(bias)) >> 64) as u64;
    let (digit, down) = if irregular {
        let half_low = half >> 1;
        (
            round(rem, (1u64 << 63) - 1).max(round(rem.wrapping_sub(half_low), u64::MAX)),
            half_low > rem,
        )
    } else {
        (
            if rem == 1 << 62 {
                2
            } else {
                round(rem, (1u64 << 63) + 6)
            },
            half > rem,
        )
    };
    let short = integral + u64::from(up);
    if up || down {
        Shortest {
            significand: short,
            exponent: k + 1,
        }
    } else {
        Shortest {
            significand: 10 * short + digit,
            exponent: k,
        }
    }
}
