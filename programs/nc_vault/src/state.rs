use anchor_lang::prelude::*;

/// Everything the vault needs to know, fixed at creation.
///
/// What is NOT here is the point: no admin, no owner, no operator, no paused flag.
#[account]
#[derive(InitSpace)]
pub struct VaultConfig {
    pub base_mint: Pubkey,
    pub receipt_mint: Pubkey,
    pub share_mint: Pubkey,
    pub vault_receipts: Pubkey,
    pub market_program: Pubkey,
    pub market_reserve: Pubkey,
    pub authority_bump: u8,
    pub config_bump: u8,
}
