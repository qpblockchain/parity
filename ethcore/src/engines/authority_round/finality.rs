// Copyright 2015-2017 Parity Technologies (UK) Ltd.
// This file is part of Parity.

// Parity is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// Parity is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with Parity.  If not, see <http://www.gnu.org/licenses/>.

//! Finality proof generation and checking.

use std::collections::{VecDeque};
use std::collections::hash_map::{HashMap, Entry};

use util::{Address, H256};

use engines::validator_set::SimpleList;

/// Error indicating unknown validator.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct UnknownValidator;

/// Rolling finality checker for authority round consensus.
/// Stores a chain of unfinalized hashes that can be pushed onto.
pub struct RollingFinality {
	headers: VecDeque<(H256, Address)>,
	signers: SimpleList,
	sign_count: HashMap<Address, usize>,
}

impl RollingFinality {
	/// Create a blank finality checker under the given validator set.
	pub fn blank(signers: Vec<Address>) -> Self {
		RollingFinality {
			headers: VecDeque::new(),
			signers: SimpleList::new(signers),
			sign_count: HashMap::new(),
		}
	}

	/// Create a rolling finality checker from an iterator of hash, signer pairs in reverse.
	/// This will take the full unfinalized subchain from the iterator.
	///
	/// Fails if any provided signature isn't part of the signers set.
	pub fn from_ancestry<I>(signers: Vec<Address>, iterable: I) -> Result<Self, UnknownValidator>
		where I: IntoIterator<Item=(H256, Address)>
	{
		let mut checker = RollingFinality::blank(signers);

		for (hash, signer) in iterable {
			if !checker.signers.contains(&signer) { return Err(UnknownValidator) }

			// break when we've got our first finalized block.
			{
				let current_signed = checker.sign_count.len();
				let would_be_finalized = (current_signed + 1) * 2 > checker.signers.len();

				let entry = checker.sign_count.entry(signer);
				if let (true, &Entry::Vacant(_)) = (would_be_finalized, &entry) {
					break
				}

				*entry.or_insert(0) += 1;
			}

			checker.headers.push_front((hash, signer));
		}

		Ok(checker)
	}

	/// Returns the last pushed hash.
	pub fn subchain_head(&self) -> Option<H256> {
		self.headers.back().map(|&(h, _)| h)
	}

	/// Get an iterator over stored hashes in order.
	pub fn unfinalized_hashes(&self) -> Iter { Iter(self.headers.iter()) }

	/// Get the validator set.
	pub fn validators(&self) -> &SimpleList { &self.validators }

	/// Push a hash onto the rolling finality checker (implying `subchain_head` == head.parent)
	///
	/// Fails if `signer` isn't a member of the active validator set.
	/// Returns a list of all newly finalized headers.
	// TODO: optimize with smallvec.
	pub fn push_hash(&mut self, head: H256, signer: Address) -> Result<Vec<H256>, UnknownValidator> {
		if !self.signers.contains(&signer) { return Err(UnknownValidator) }

		self.headers.push_back((head, signer));
		*self.sign_count.entry(signer).or_insert(0) += 1;

		let mut newly_finalized = Vec::new();

		while self.sign_count.len() * 2 > self.signers.len() {
			let (hash, signer) = self.headers.pop_front()
				.expect("headers length always greater than sign count length; qed");

			newly_finalized.push(hash);

			match self.sign_count.entry(signer) {
				Entry::Occupied(mut entry) => {
					// decrement count for this signer and purge on zero.
					*entry.get_mut() -= 1;

					if *entry.get() == 0 {
						entry.remove();
					}
				}
				Entry::Vacant(_) => panic!("all hashes in `header` should have an entry in `sign_count` for their signer; qed"),
			}
		}

		Ok(newly_finalized)
	}
}

pub struct Iter<'a>(::std::collections::vec_deque::Iter<'a, (H256, Address)>);

impl<'a> Iterator for Iter<'a> {
	type Item = H256;

	fn next(&mut self) -> Option<H256> {
		self.0.next().map(|&(h, _)| h)
	}
}

#[cfg(test)]
mod tests {
	use util::{Address, H256};
	use super::RollingFinality;

	#[test]
	fn rejects_unknown_signer() {
		let signers = (0..3).map(|_| Address::random()).collect();
		let mut finality = RollingFinality::blank(signers);
		assert!(finality.push_hash(H256::random(), Address::random()).is_err());
	}

	#[test]
	fn finalize_multiple() {
		let signers: Vec<_> = (0..6).map(|_| Address::random()).collect();

		let mut finality = RollingFinality::blank(signers.clone());
		let hashes: Vec<_> = (0..7).map(|_| H256::random()).collect();

		// 3 / 6 signers is < 51% so no finality.
		for (i, hash) in hashes.iter().take(6).cloned().enumerate() {
			let i = i % 3;
			assert!(finality.push_hash(hash, signers[i]).unwrap().len() == 0);
		}

		// after pushing a block signed by a fourth validator, the first four
		// blocks of the unverified chain become verified.
		assert_eq!(finality.push_hash(hashes[6], signers[4]).unwrap(),
			vec![hashes[0], hashes[1], hashes[2], hashes[3]]);
	}

	#[test]
	fn from_ancestry() {
		let signers: Vec<_> = (0..6).map(|_| Address::random()).collect();
		let hashes: Vec<_> = (0..12).map(|i| (H256::random(), signers[i % 6])).collect();

		let finality = RollingFinality::from_ancestry(signers.clone(),
			hashes.iter().rev().cloned()).unwrap();

		assert_eq!(finality.unfinalized_hashes().count(), 3);
		assert_eq!(finality.subchain_head(), Some(hashes[11].0));
	}
}
