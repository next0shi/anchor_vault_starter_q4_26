use anchor_lang::prelude::*;

#[constant]
pub const CONFIG_SEED: &[u8] = b"config";

#[constant]
pub const AUTHORITY_SEED: &[u8] = b"authority";

#[constant]
pub const SHARES_SEED: &[u8] = b"shares";

/// Shares carry this many more decimals than receipts. Together with the virtual
/// amounts below it makes the first-depositor "inflation" attack uneconomical.
pub const DECIMALS_OFFSET: u8 = 3;

/// 10^DECIMALS_OFFSET: virtual shares counted in the share price.
pub const VIRTUAL_SHARES: u64 = 1_000;

/// Virtual receipts counted in the share price.
pub const VIRTUAL_RECEIPTS: u64 = 1;
