//! Share maths. Shares are priced in RECEIPT tokens, never in base asset, so nothing
//! here needs an oracle or the market's exchange rate.
//!
//! Both directions round DOWN, so rounding dust always stays in the vault.

use crate::{
    constants::{VIRTUAL_RECEIPTS, VIRTUAL_SHARES},
    error::VaultError,
};
use anchor_lang::prelude::*;

/// Shares to mint for `receipts_in` freshly deposited receipts.
/// `total_receipts` must be the vault's balance from BEFORE this deposit.
pub fn shares_for_receipts(receipts_in: u64, total_shares: u64, total_receipts: u64) -> Result<u64> {
    mul_div(
        receipts_in,
        total_shares
            .checked_add(VIRTUAL_SHARES)
            .ok_or(VaultError::MathOverflow)?,
        total_receipts
            .checked_add(VIRTUAL_RECEIPTS)
            .ok_or(VaultError::MathOverflow)?,
    )
}

/// Receipts owed for `shares`, as a pro-rata slice of everything the vault holds.
pub fn receipts_for_shares(shares: u64, total_shares: u64, total_receipts: u64) -> Result<u64> {
    mul_div(
        shares,
        total_receipts
            .checked_add(VIRTUAL_RECEIPTS)
            .ok_or(VaultError::MathOverflow)?,
        total_shares
            .checked_add(VIRTUAL_SHARES)
            .ok_or(VaultError::MathOverflow)?,
    )
}

fn mul_div(a: u64, b: u64, c: u64) -> Result<u64> {
    let v = (a as u128)
        .checked_mul(b as u128)
        .ok_or(VaultError::MathOverflow)?
        .checked_div(c as u128)
        .ok_or(VaultError::MathOverflow)?;
    u64::try_from(v).map_err(|_| error!(VaultError::MathOverflow))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_deposit_then_full_redeem_returns_exactly_what_went_in() {
        let shares = shares_for_receipts(1_000_000, 0, 0).unwrap();
        assert_eq!(shares, 1_000_000 * 1_000);
        let back = receipts_for_shares(shares, shares, 1_000_000).unwrap();
        assert_eq!(back, 1_000_000);
    }

    #[test]
    fn ratio_stays_exactly_1000_to_1_in_normal_flows() {
        let (mut shares, mut receipts) = (0u64, 0u64);
        for deposit in [1u64, 7, 1_000, 123_456_789, 42] {
            let minted = shares_for_receipts(deposit, shares, receipts).unwrap();
            assert_eq!(minted, deposit * 1_000);
            shares += minted;
            receipts += deposit;
        }
    }

    #[test]
    fn rounding_never_favours_the_user() {
        // Deposit, then immediately redeem what was minted: never get more than you put in.
        for (s, r) in [(0u64, 0u64), (1_000, 1), (5_000_000, 1_234), (999, 777_777_777)] {
            for deposit in [1u64, 2, 3, 10, 999, 1_001, 1_000_000] {
                let minted = shares_for_receipts(deposit, s, r).unwrap();
                let back = receipts_for_shares(minted, s + minted, r + deposit).unwrap();
                assert!(back <= deposit, "got {back} back for {deposit} at ({s}, {r})");
            }
        }
    }

    #[test]
    fn donation_attack_is_uneconomical() {
        let attacker_in = 1u64;
        let donation = 1_000_000_000u64;
        let victim_in = 500_000_000u64;

        // Attacker deposits the minimum, then donates straight to the vault.
        let attacker_shares = shares_for_receipts(attacker_in, 0, 0).unwrap();
        let (mut shares, mut receipts) = (attacker_shares, attacker_in + donation);

        // The victim still gets shares (plain maths would have rounded this to zero).
        let victim_shares = shares_for_receipts(victim_in, shares, receipts).unwrap();
        assert!(victim_shares > 0);
        shares += victim_shares;
        receipts += victim_in;

        let victim_out = receipts_for_shares(victim_shares, shares, receipts).unwrap();
        let attacker_out = receipts_for_shares(attacker_shares, shares, receipts).unwrap();

        assert!(victim_out >= victim_in / 1_000 * 998, "victim keeps at least 99.8%");
        assert!(attacker_out < attacker_in + donation, "attacker loses money");
    }
}
