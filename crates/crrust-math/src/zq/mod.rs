#![warn(missing_docs, unused_imports)]

//! Ring operations for moduli up to 62 bits.

pub mod nfl;
pub mod ntt;

use crrust_util::is_prime;
use itertools::{izip, Itertools};
use num_bigint::BigUint;
use num_traits::cast::ToPrimitive;
use rand::{distributions::Uniform, thread_rng, Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

/// Structure holding a modulus up to 62 bits.
#[derive(Debug, Clone, PartialEq)]
pub struct Modulus {
	p: u64,
	barrett_hi: u64,
	barrett_lo: u64,
	leading_zeros: u32,
	supports_opt: bool,
	distribution: Uniform<u64>,
}

impl Modulus {
	/// Create a modulus from an integer of at most 62 bits.
	pub fn new(p: u64) -> std::option::Option<Self> {
		if p < 2 || (p >> 62) != 0 {
			None
		} else {
			let barrett = ((BigUint::from(1u64) << 128usize) / p).to_u128().unwrap(); // 2^128 / p

			Some(Self {
				p,
				barrett_hi: (barrett >> 64) as u64,
				barrett_lo: barrett as u64,
				leading_zeros: p.leading_zeros(),
				supports_opt: nfl::supports_opt(p),
				distribution: Uniform::from(0..p),
			})
		}
	}

	/// Returns the value of the modulus.
	pub fn modulus(&self) -> u64 {
		self.p
	}

	/// Modular addition of a and b in constant time.
	///
	/// Aborts if a >= p or b >= p in debug mode.
	pub fn add(&self, a: u64, b: u64) -> u64 {
		debug_assert!(a < self.p && b < self.p);

		Self::reduce1(a + b, self.p)
	}

	/// Modular subtraction of a and b in constant time.
	///
	/// Aborts if a >= p or b >= p in debug mode.
	pub fn sub(&self, a: u64, b: u64) -> u64 {
		debug_assert!(a < self.p && b < self.p);

		Self::reduce1(a + self.p - b, self.p)
	}

	/// Modular multiplication of a and b in constant time.
	///
	/// Aborts if a >= p or b >= p in debug mode.
	pub fn mul(&self, a: u64, b: u64) -> u64 {
		debug_assert!(a < self.p && b < self.p);

		self.reduce_u128((a as u128) * (b as u128))
	}

	/// Optimized modular multiplication of a and b in constant time.
	///
	/// Aborts if a >= p or b >= p in debug mode.
	pub fn mul_opt(&self, a: u64, b: u64) -> u64 {
		debug_assert!(self.supports_opt);
		debug_assert!(a < self.p && b < self.p);

		self.reduce_opt_u128((a as u128) * (b as u128))
	}

	/// Modular negation in constant time.
	///
	/// Aborts if a >= p in debug mode.
	pub fn neg(&self, a: u64) -> u64 {
		debug_assert!(a < self.p);
		Self::reduce1(self.p - a, self.p)
	}

	/// Compute the Shoup representation of a.
	///
	/// Aborts if a >= p in debug mode.
	pub fn shoup(&self, a: u64) -> u64 {
		debug_assert!(a < self.p);

		(((a as u128) << 64) / (self.p as u128)) as u64
	}

	/// Shoup multiplication of a and b in constant time.
	///
	/// Aborts if b >= p or b_shoup != shoup(b) in debug mode.
	pub fn mul_shoup(&self, a: u64, b: u64, b_shoup: u64) -> u64 {
		Self::reduce1(self.lazy_mul_shoup(a, b, b_shoup), self.p)
	}

	/// Lazy Shoup multiplication of a and b in constant time.
	/// The output is in the interval [0, 2 * p).
	///
	/// Aborts if b >= p or b_shoup != shoup(b) in debug mode.
	pub fn lazy_mul_shoup(&self, a: u64, b: u64, b_shoup: u64) -> u64 {
		debug_assert!(b < self.p);
		debug_assert_eq!(b_shoup, self.shoup(b));

		let q = ((a as u128) * (b_shoup as u128)) >> 64;
		let r = ((a as u128) * (b as u128) - q * (self.p as u128)) as u64;

		debug_assert!(r < 2 * self.p);

		r
	}

	/// Modular addition of vectors in place in constant time.
	///
	/// Aborts if a and b differ in size, and if any of their values is >= p in debug mode.
	pub fn add_vec(&self, a: &mut [u64], b: &[u64]) {
		debug_assert_eq!(a.len(), b.len());

		izip!(a.iter_mut(), b.iter()).for_each(|(ai, bi)| *ai = self.add(*ai, *bi));
	}

	/// Modular subtraction of vectors in place in constant time.
	///
	/// Aborts if a and b differ in size, and if any of their values is >= p in debug mode.
	pub fn sub_vec(&self, a: &mut [u64], b: &[u64]) {
		debug_assert_eq!(a.len(), b.len());

		izip!(a.iter_mut(), b.iter()).for_each(|(ai, bi)| *ai = self.sub(*ai, *bi));
	}

	/// Modular multiplication of vectors in place in constant time.
	///
	/// Aborts if a and b differ in size, and if any of their values is >= p in debug mode.
	pub fn mul_vec(&self, a: &mut [u64], b: &[u64]) {
		debug_assert_eq!(a.len(), b.len());

		if self.supports_opt {
			izip!(a.iter_mut(), b.iter()).for_each(|(ai, bi)| *ai = self.mul_opt(*ai, *bi));
		} else {
			izip!(a.iter_mut(), b.iter()).for_each(|(ai, bi)| *ai = self.mul(*ai, *bi));
		}
	}

	/// Compute the Shoup representation of a vector.
	///
	/// Aborts if any of the values of the vector is >= p in debug mode.
	pub fn shoup_vec(&self, a: &[u64]) -> Vec<u64> {
		let mut a_shoup = Vec::with_capacity(a.len());
		a.iter().for_each(|ai| a_shoup.push(self.shoup(*ai)));
		a_shoup
	}

	/// Shoup modular multiplication of vectors in place in constant time.
	///
	/// Aborts if a and b differ in size, and if any of their values is >= p in debug mode.
	pub fn mul_shoup_vec(&self, a: &mut [u64], b: &[u64], b_shoup: &[u64]) {
		debug_assert_eq!(a.len(), b.len());
		debug_assert_eq!(a.len(), b_shoup.len());
		debug_assert_eq!(&b_shoup, &self.shoup_vec(b));

		izip!(a.iter_mut(), b.iter(), b_shoup.iter())
			.for_each(|(ai, bi, bi_shoup)| *ai = self.mul_shoup(*ai, *bi, *bi_shoup));
	}

	/// Reduce a vector in place in constant time.
	pub fn reduce_vec(&self, a: &mut [u64]) {
		a.iter_mut().for_each(|ai| *ai = self.reduce(*ai));
	}

	/// Reduce a vector in constant time.
	pub fn reduce_vec_new(&self, a: &[u64]) -> Vec<u64> {
		let mut b = a.to_vec();
		b.iter_mut().for_each(|bi| *bi = self.reduce(*bi));
		b
	}

	/// Modular negation of a vector in place in constant time.
	///
	/// Aborts if any of the values in the vector is >= p in debug mode.
	pub fn neg_vec(&self, a: &mut [u64]) {
		izip!(a.iter_mut()).for_each(|ai| *ai = self.neg(*ai));
	}

	/// Modular exponentiation in variable time.
	///
	/// Aborts if a >= p or n >= p in debug mode.
	pub fn pow(&self, a: u64, n: u64) -> u64 {
		debug_assert!(a < self.p && n < self.p);

		if n == 0 {
			1
		} else if n == 1 {
			a
		} else {
			let mut r = a;
			let mut i = (62 - n.leading_zeros()) as isize;
			while i >= 0 {
				r = self.mul(r, r);
				if (n >> i) & 1 == 1 {
					r = self.mul(r, a);
				}
				i -= 1;
			}
			r
		}
	}

	/// Modular inversion in variable time.
	///
	/// Returns None if p is not prime or a = 0.
	/// Aborts if a >= p in debug mode.
	pub fn inv(&self, a: u64) -> std::option::Option<u64> {
		if !is_prime(self.p) || a == 0 {
			None
		} else {
			let r = self.pow(a, self.p - 2);
			debug_assert_eq!(self.mul(a, r), 1);
			Some(r)
		}
	}

	/// Modular reduction of a u128 in constant time.
	pub fn reduce_u128(&self, a: u128) -> u64 {
		Self::reduce1(self.lazy_reduce_u128(a), self.p)
	}

	/// Modular reduction of a u64 in constant time.
	pub fn reduce(&self, a: u64) -> u64 {
		Self::reduce1(self.lazy_reduce(a), self.p)
	}

	/// Optimized modular reduction of a u128 in constant time.
	fn reduce_opt_u128(&self, a: u128) -> u64 {
		debug_assert!(self.supports_opt);
		Self::reduce1(self.lazy_reduce_opt_u128(a), self.p)
	}

	/// Optimized modular reduction of a u64 in constant time.
	pub fn reduce_opt(&self, a: u64) -> u64 {
		Self::reduce1(self.lazy_reduce_opt(a), self.p)
	}

	/// Return x mod p in constant time.
	///
	/// Aborts if x >= 2 * p in debug mode.
	fn reduce1(x: u64, p: u64) -> u64 {
		debug_assert!(p >> 63 == 0);
		debug_assert!(x < 2 * p);

		let (y, _) = x.overflowing_sub(p);
		let xp = x ^ p;
		let yp = y ^ p;
		let xy = xp ^ yp;
		let xxy = x ^ xy;
		let xxy = xxy >> 63;
		let (c, _) = xxy.overflowing_sub(1);
		let r = (c & y) | ((!c) & x);

		debug_assert_eq!(r, x % p);

		r
	}

	/// Lazy modular reduction of a in constant time.
	/// The output is in the interval [0, 2 * p).
	pub fn lazy_reduce_u128(&self, a: u128) -> u64 {
		let a_lo = a as u64;
		let a_hi = (a >> 64) as u64;
		let p_lo_lo = ((a_lo as u128) * (self.barrett_lo as u128)) >> 64;
		let p_hi_lo = (a_hi as u128) * (self.barrett_lo as u128);
		let p_lo_hi = (a_lo as u128) * (self.barrett_hi as u128);

		let q = ((p_lo_hi + p_hi_lo + p_lo_lo) >> 64) + (a_hi as u128) * (self.barrett_hi as u128);
		let r = (a - q * (self.p as u128)) as u64;

		debug_assert!((r as u128) < 2 * (self.p as u128));
		debug_assert_eq!(r % self.p, (a % (self.p as u128)) as u64);

		r
	}

	/// Lazy modular reduction of a in constant time.
	/// The output is in the interval [0, 2 * p).
	pub fn lazy_reduce(&self, a: u64) -> u64 {
		let p_lo_lo = ((a as u128) * (self.barrett_lo as u128)) >> 64;
		let p_lo_hi = (a as u128) * (self.barrett_hi as u128);

		let q = (p_lo_hi + p_lo_lo) >> 64;
		let r = (a as u128 - q * (self.p as u128)) as u64;

		debug_assert!((r as u128) < 2 * (self.p as u128));
		debug_assert_eq!(r % self.p, a % self.p);

		r
	}

	/// Lazy optimized modular reduction of a in constant time.
	/// The output is in the interval [0, 2 * p).
	///
	/// Aborts if the input is >= 2 * p in debug mode.
	fn lazy_reduce_opt_u128(&self, a: u128) -> u64 {
		debug_assert!(a < (self.p as u128) * (self.p as u128));

		let q = (((self.barrett_lo as u128) * (a >> 64)) + (a << self.leading_zeros)) >> 64;
		let r = (a - q * (self.p as u128)) as u64;

		debug_assert!((r as u128) < 2 * (self.p as u128));
		debug_assert_eq!(r % self.p, (a % (self.p as u128)) as u64);

		r
	}

	/// Lazy optimized modular reduction of a in constant time.
	/// The output is in the interval [0, 2 * p).
	///
	/// Aborts if the input is >= 2 * p in debug mode.
	fn lazy_reduce_opt(&self, a: u64) -> u64 {
		let q = a >> (64 - self.leading_zeros);
		let r = ((a as u128) - (q as u128) * (self.p as u128)) as u64;

		debug_assert!((r as u128) < 2 * (self.p as u128));
		debug_assert_eq!(r % self.p, a % self.p);

		r
	}

	/// Returns a random vector and a seed that generated that random element.
	///
	/// Panics if the size is 0.
	pub fn random_vec(&self, size: usize) -> Vec<u64> {
		let mut seed = <ChaCha8Rng as SeedableRng>::Seed::default();
		thread_rng().fill(&mut seed);
		self.random_vec_from_seed(size, seed)
	}

	/// Returns a random vector generated deterministically from a seed.
	///
	/// Panics if the size is 0.
	pub fn random_vec_from_seed(
		&self,
		size: usize,
		seed: <ChaCha8Rng as SeedableRng>::Seed,
	) -> Vec<u64> {
		assert_ne!(size, 0);
		let rng = rand_chacha::ChaCha8Rng::from_seed(seed);
		rng.sample_iter(self.distribution).take(size).collect_vec()
	}
}

#[cfg(test)]
mod tests {
	use super::{nfl, Modulus};
	use crrust_util::catch_unwind;
	use itertools::{izip, Itertools};
	use proptest::collection::vec as prop_vec;
	use proptest::prelude::{any, BoxedStrategy, Just, Strategy};
	use rand::{thread_rng, Rng, RngCore, SeedableRng};
	use rand_chacha::ChaCha8Rng;

	// Utility functions for the proptests.

	fn valid_moduli() -> impl Strategy<Value = Modulus> {
		any::<u64>().prop_filter_map("filter invalid moduli", |p| Modulus::new(p))
	}

	fn vecs() -> BoxedStrategy<(Vec<u64>, Vec<u64>)> {
		prop_vec(any::<u64>(), 1..100)
			.prop_flat_map(|vec| {
				let len = vec.len();
				(Just(vec), prop_vec(any::<u64>(), len))
			})
			.boxed()
	}

	proptest! {
		#[test]
		fn test_constructor(p: u64) {
			// 63 and 64-bit integers do not work.
			prop_assert!(Modulus::new(p | (1u64 << 62)).is_none());
			prop_assert!(Modulus::new(p | (1u64 << 63)).is_none());

			// p = 0 & 1 do not work.
			prop_assert!(Modulus::new(0u64).is_none());
			prop_assert!(Modulus::new(1u64).is_none());

			// Otherwise, all moduli should work.
			prop_assume!(p >> 2 >= 2);
			prop_assert!(Modulus::new(p >> 2).is_some_and(|q| q.modulus() == p >> 2));
		}

		#[test]
		fn test_neg(p in valid_moduli(), mut a: u64,  mut q: u64) {
			a = p.reduce(a);
			prop_assert_eq!(p.neg(a), (p.modulus() - a) % p.modulus());

			q = (q % (u64::MAX - p.modulus())) + 1 + p.modulus(); // q > p
			prop_assert!(catch_unwind(|| p.neg(q)).is_err());
		}

		#[test]
		fn test_add(p in valid_moduli(), mut a: u64, mut b: u64, mut q: u64) {
			a = p.reduce(a);
			b = p.reduce(b);
			prop_assert_eq!(p.add(a, b), (a + b) % p.modulus());

			q = (q % (u64::MAX - p.modulus())) + 1 + p.modulus(); // q > p
			prop_assert!(catch_unwind(|| p.add(q, a)).is_err());
			prop_assert!(catch_unwind(|| p.add(a, q)).is_err());
		}

		#[test]
		fn test_sub(p in valid_moduli(), mut a: u64, mut b: u64, mut q: u64) {
			a = p.reduce(a);
			b = p.reduce(b);
			prop_assert_eq!(p.sub(a, b), (a + p.modulus() - b) % p.modulus());

			q = (q % (u64::MAX - p.modulus())) + 1 + p.modulus(); // q > p
			prop_assert!(catch_unwind(|| p.sub(q, a)).is_err());
			prop_assert!(catch_unwind(|| p.sub(a, q)).is_err());
		}

		#[test]
		fn test_mul(p in valid_moduli(), mut a: u64, mut b: u64, mut q: u64) {
			a = p.reduce(a);
			b = p.reduce(b);
			prop_assert_eq!(p.mul(a, b) as u128, ((a as u128) * (b as u128)) % (p.modulus() as u128));

			q = (q % (u64::MAX - p.modulus())) + 1 + p.modulus(); // q > p
			prop_assert!(catch_unwind(|| p.mul(q, a)).is_err());
			prop_assert!(catch_unwind(|| p.mul(a, q)).is_err());
		}

		#[test]
		fn test_mul_shoup(p in valid_moduli(), mut a: u64, mut b: u64, mut q: u64) {
			a = p.reduce(a);
			b = p.reduce(b);
			let b_shoup = p.shoup(b);
			prop_assert_eq!(p.mul_shoup(a, b, b_shoup) as u128, ((a as u128) * (b as u128)) % (p.modulus() as u128));

			q = (q % (u64::MAX - p.modulus())) + 1 + p.modulus(); // q > p
			prop_assert!(catch_unwind(|| p.shoup(q)).is_err());
			prop_assert!(catch_unwind(|| p.mul_shoup(a, q, b_shoup)).is_err());

			prop_assume!(a != b);
			prop_assert!(catch_unwind(|| p.mul_shoup(a, a, b_shoup)).is_err());
		}

		#[test]
		fn test_reduce(p in valid_moduli(), a: u64) {
			prop_assert_eq!(p.reduce(a), a % p.modulus());
		}

		#[test]
		fn test_reduce_128(p in valid_moduli(), a: u128) {
			prop_assert_eq!(p.reduce_u128(a) as u128, a % (p.modulus() as u128));
		}

		#[test]
		fn test_add_vec(p in valid_moduli(), (mut a, mut b) in vecs()) {
			p.reduce_vec(&mut a);
			p.reduce_vec(&mut b);
			let c = a.clone();
			p.add_vec(&mut a, &b);
			prop_assert_eq!(a, izip!(b.iter(), c.iter()).map(|(bi, ci)| p.add(*bi, *ci)).collect_vec());
		}

		#[test]
		fn test_sub_vec(p in valid_moduli(), (mut a, mut b) in vecs()) {
			p.reduce_vec(&mut a);
			p.reduce_vec(&mut b);
			let c = a.clone();
			p.sub_vec(&mut a, &b);
			prop_assert_eq!(a, izip!(b.iter(), c.iter()).map(|(bi, ci)| p.sub(*ci, *bi)).collect_vec());
		}

		#[test]
		fn test_mul_vec(p in valid_moduli(), (mut a, mut b) in vecs()) {
			p.reduce_vec(&mut a);
			p.reduce_vec(&mut b);
			let c = a.clone();
			p.mul_vec(&mut a, &b);
			prop_assert_eq!(a, izip!(b.iter(), c.iter()).map(|(bi, ci)| p.mul(*ci, *bi)).collect_vec());
		}

		#[test]
		fn test_mul_shoup_vec(p in valid_moduli(), (mut a, mut b) in vecs()) {
			p.reduce_vec(&mut a);
			p.reduce_vec(&mut b);
			let b_shoup = p.shoup_vec(&b);
			let c = a.clone();
			p.mul_shoup_vec(&mut a, &b, &b_shoup);
			prop_assert_eq!(a, izip!(b.iter(), c.iter()).map(|(bi, ci)| p.mul(*ci, *bi)).collect_vec());
		}

		#[test]
		fn test_reduce_vec(p in valid_moduli(), a: Vec<u64>) {
			let mut b = a.clone();
			p.reduce_vec(&mut b);
			prop_assert_eq!(b, a.iter().map(|ai| p.reduce(*ai)).collect_vec());
		}

		#[test]
		fn test_neg_vec(p in valid_moduli(), mut a: Vec<u64>) {
			p.reduce_vec(&mut a);
			let mut b = a.clone();
			p.neg_vec(&mut b);
			prop_assert_eq!(b, a.iter().map(|ai| p.neg(*ai)).collect_vec());
		}

		#[test]
		fn test_random_vec(p in valid_moduli(), size in 1..1000usize) {
			let v = p.random_vec(size);
			prop_assert_eq!(v.len(), size);

			let mut seed = <ChaCha8Rng as SeedableRng>::Seed::default();
			thread_rng().fill(&mut seed);
			let x = p.random_vec_from_seed(size, seed.clone());
			let y = p.random_vec_from_seed(size, seed.clone());
			prop_assert_eq!(x.len(), size);
			prop_assert_eq!(&x, &y);

			thread_rng().fill(&mut seed);
			let y = p.random_vec_from_seed(size, seed.clone());
			prop_assert_eq!(y.len(), size);
			if p.modulus().leading_zeros() <= 30 {
				prop_assert_ne!(x, y); // This will hold with probability at least 2^(-30)
			}

			prop_assert!(catch_unwind(|| p.random_vec(0)).is_err());
			prop_assert!(catch_unwind(|| p.random_vec_from_seed(0, seed)).is_err());
		}
	}

	// TODO: Make a proptest.
	#[test]
	fn test_mul_opt() {
		let ntests = 100;
		let mut rng = rand::thread_rng();

		for p in [4611686018326724609] {
			let q = Modulus::new(p).unwrap();
			assert!(nfl::supports_opt(p));

			assert_eq!(q.mul_opt(0, 1), 0);
			assert_eq!(q.mul_opt(1, 1), 1);
			assert_eq!(q.mul_opt(2 % p, 3 % p), 6 % p);
			assert_eq!(q.mul_opt(p - 1, 1), p - 1);
			assert_eq!(q.mul_opt(p - 1, 2 % p), p - 2);

			assert!(catch_unwind(|| q.mul_opt(p, 1)).is_err());
			assert!(catch_unwind(|| q.mul_opt(p << 1, 1)).is_err());
			assert!(catch_unwind(|| q.mul_opt(0, p)).is_err());
			assert!(catch_unwind(|| q.mul_opt(0, p << 1)).is_err());

			for _ in 0..ntests {
				let a = rng.next_u64() % p;
				let b = rng.next_u64() % p;
				assert_eq!(
					q.mul_opt(a, b),
					(((a as u128) * (b as u128)) % (p as u128)) as u64
				);
			}
		}
	}

	// TODO: Make a proptest.
	#[test]
	fn test_pow() {
		let ntests = 10;
		let mut rng = rand::thread_rng();

		for p in [2u64, 3, 17, 1987, 4611686018326724609] {
			let q = Modulus::new(p).unwrap();

			assert_eq!(q.pow(p - 1, 0), 1);
			assert_eq!(q.pow(p - 1, 1), p - 1);
			assert_eq!(q.pow(p - 1, 2 % p), 1);
			assert_eq!(q.pow(1, p - 2), 1);
			assert_eq!(q.pow(1, p - 1), 1);

			assert!(catch_unwind(|| q.pow(p, 1)).is_err());
			assert!(catch_unwind(|| q.pow(p << 1, 1)).is_err());
			assert!(catch_unwind(|| q.pow(0, p)).is_err());
			assert!(catch_unwind(|| q.pow(0, p << 1)).is_err());

			for _ in 0..ntests {
				let a = rng.next_u64() % p;
				let b = (rng.next_u64() % p) % 1000;
				let mut c = b;
				let mut r = 1;
				while c > 0 {
					r = q.mul(r, a);
					c -= 1;
				}
				assert_eq!(q.pow(a, b), r);
			}
		}
	}

	// TODO: Make a proptest.
	#[test]
	fn test_inv() {
		let ntests = 100;
		let mut rng = rand::thread_rng();

		for p in [2u64, 3, 17, 1987, 4611686018326724609] {
			let q = Modulus::new(p).unwrap();

			assert!(q.inv(0).is_none());
			assert_eq!(q.inv(1).unwrap(), 1);
			assert_eq!(q.inv(p - 1).unwrap(), p - 1);

			assert!(catch_unwind(|| q.inv(p)).is_err());
			assert!(catch_unwind(|| q.inv(p << 1)).is_err());

			for _ in 0..ntests {
				let a = rng.next_u64() % p;
				let b = q.inv(a);

				if a == 0 {
					assert!(b.is_none())
				} else {
					assert!(b.is_some());
					assert_eq!(q.mul(a, b.unwrap()), 1)
				}
			}
		}
	}
}