use anchor_lang::prelude::*;

use crate::constants::MINER_TREE_CAP;
use crate::errors::BitcoinError;

/// Number of usable (1-indexed) leaves in the Fenwick tree.
const N: u32 = (MINER_TREE_CAP - 1) as u32;

/// Zero-copy global active-miner index for the weighted lottery.
///
/// `nodes` is a 1-indexed Fenwick (binary indexed) tree of per-slot hashrate.
/// `slot_mint` maps an active slot to the NFT mint occupying it, so the crank
/// can record the drawn winner without the winner's accounts being passed in.
/// `free` is a stack of reusable slots freed on deactivation.
#[account(zero_copy)]
#[repr(C)]
pub struct MinerTree {
    /// Sum of all active hashrate (equals nodes prefix_sum(N)).
    pub total: u64,
    /// Next never-before-used slot (starts at 1).
    pub high_water: u32,
    /// Number of entries in the free stack.
    pub free_len: u32,
    pub bump: u8,
    pub _pad: [u8; 7],
    pub nodes: [u64; MINER_TREE_CAP],
    pub slot_mint: [Pubkey; MINER_TREE_CAP],
    pub free: [u32; MINER_TREE_CAP],
}

impl MinerTree {
    pub fn initialize(&mut self) {
        self.total = 0;
        self.high_water = 1;
        self.free_len = 0;
        self.bump = 0;
        // Account is zeroed at init; arrays are already 0.
    }

    /// Apply a signed delta to leaf `i` (1-indexed) and the running total.
    fn fenwick_add(&mut self, i: u32, delta: i128) -> Result<()> {
        require!(i >= 1 && i <= N, BitcoinError::InvalidSlot);
        let mut idx = i;
        while idx <= N {
            let cur = self.nodes[idx as usize] as i128;
            let next = cur
                .checked_add(delta)
                .ok_or(BitcoinError::MathOverflow)?;
            require!(next >= 0, BitcoinError::MathOverflow);
            self.nodes[idx as usize] = next as u64;
            idx += idx & idx.wrapping_neg();
        }
        let new_total = (self.total as i128)
            .checked_add(delta)
            .ok_or(BitcoinError::MathOverflow)?;
        require!(new_total >= 0, BitcoinError::MathOverflow);
        self.total = new_total as u64;
        Ok(())
    }

    /// Allocate a slot for `mint`, reusing freed slots first.
    pub fn allocate_slot(&mut self, mint: Pubkey) -> Result<u32> {
        let slot = if self.free_len > 0 {
            self.free_len -= 1;
            self.free[self.free_len as usize]
        } else {
            let s = self.high_water;
            require!(s <= N, BitcoinError::TreeFull);
            self.high_water = s.checked_add(1).ok_or(BitcoinError::MathOverflow)?;
            s
        };
        self.slot_mint[slot as usize] = mint;
        Ok(slot)
    }

    /// Insert an active miner with the given hashrate, returning its slot.
    pub fn insert(&mut self, mint: Pubkey, hashrate: u64) -> Result<u32> {
        let slot = self.allocate_slot(mint)?;
        self.fenwick_add(slot, hashrate as i128)?;
        Ok(slot)
    }

    /// Remove an active miner from `slot` with its `hashrate`.
    pub fn remove(&mut self, slot: u32, hashrate: u64) -> Result<()> {
        require!(slot >= 1 && slot <= N, BitcoinError::InvalidSlot);
        self.fenwick_add(slot, -(hashrate as i128))?;
        self.slot_mint[slot as usize] = Pubkey::default();
        require!(
            (self.free_len as usize) < MINER_TREE_CAP,
            BitcoinError::TreeFull
        );
        self.free[self.free_len as usize] = slot;
        self.free_len += 1;
        Ok(())
    }

    /// Find the slot whose cumulative hashrate range contains `target`.
    /// `target` must be in `[0, total)`.
    pub fn find_by_prefix(&self, mut target: u128) -> Result<u32> {
        require!(self.total > 0, BitcoinError::NoActiveMiners);
        if target >= self.total as u128 {
            target = (self.total - 1) as u128;
        }
        let mut pos: u32 = 0;
        let mut bit: u32 = if N == 0 {
            0
        } else {
            1u32 << (31 - N.leading_zeros())
        };
        while bit != 0 {
            let next = pos + bit;
            if next <= N && (self.nodes[next as usize] as u128) <= target {
                pos = next;
                target -= self.nodes[next as usize] as u128;
            }
            bit >>= 1;
        }
        let slot = pos + 1;
        require!(slot >= 1 && slot <= N, BitcoinError::InvalidSlot);
        Ok(slot)
    }

    pub fn mint_at(&self, slot: u32) -> Pubkey {
        self.slot_mint[slot as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_tree() -> Box<MinerTree> {
        // The tree is ~112KB; box it to keep it off the stack. MinerTree is a
        // zero-copy Pod type, so an all-zero value is valid.
        let mut t: Box<MinerTree> = Box::new(unsafe { std::mem::zeroed() });
        t.initialize();
        t
    }

    fn mint(i: u8) -> Pubkey {
        Pubkey::new_from_array([i; 32])
    }

    #[test]
    fn insert_remove_tracks_total() {
        let mut t = new_tree();
        let s1 = t.insert(mint(1), 10).unwrap();
        let s2 = t.insert(mint(2), 40).unwrap();
        assert_eq!(t.total, 50);
        t.remove(s1, 10).unwrap();
        assert_eq!(t.total, 40);
        // Freed slot is reused.
        let s3 = t.insert(mint(3), 5).unwrap();
        assert_eq!(s3, s1);
        assert_eq!(t.total, 45);
        let _ = s2;
    }

    #[test]
    fn prefix_search_maps_ranges_to_slots() {
        let mut t = new_tree();
        let a = t.insert(mint(1), 10).unwrap(); // range [0,10)
        let b = t.insert(mint(2), 20).unwrap(); // range [10,30)
        let c = t.insert(mint(3), 70).unwrap(); // range [30,100)
        assert_eq!(t.find_by_prefix(0).unwrap(), a);
        assert_eq!(t.find_by_prefix(9).unwrap(), a);
        assert_eq!(t.find_by_prefix(10).unwrap(), b);
        assert_eq!(t.find_by_prefix(29).unwrap(), b);
        assert_eq!(t.find_by_prefix(30).unwrap(), c);
        assert_eq!(t.find_by_prefix(99).unwrap(), c);
    }

    #[test]
    fn weighted_draw_is_proportional() {
        let mut t = new_tree();
        t.insert(mint(1), 10).unwrap();
        t.insert(mint(2), 90).unwrap();
        let mut hits = [0u32; 3];
        for r in 0..1000u128 {
            let target = (r * 7919) % t.total as u128;
            let slot = t.find_by_prefix(target).unwrap();
            hits[slot as usize] += 1;
        }
        // Slot 2 (weight 90) should win far more often than slot 1 (weight 10).
        assert!(hits[2] > hits[1] * 5);
    }
}
