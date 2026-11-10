use crate::{
    constants::{AUTHORITY_SEED, CONFIG_SEED, DECIMALS_OFFSET, SHARES_SEED},
    error::VaultError,
    state::VaultConfig,
};
use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token::{Mint, Token, TokenAccount},
};
use mock_market::program::MockMarket;

/// Anyone can create a vault for a market. There is no admin to ask, and every
/// parameter is part of the config address, so nobody can squat a bad config on
/// someone else's address.
#[derive(Accounts)]
pub struct InitializeVault<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    pub base_mint: Box<Account<'info, Mint>>,
    pub receipt_mint: Box<Account<'info, Mint>>,

    /// The only external program this vault will ever call.
    pub market_program: Program<'info, MockMarket>,

    /// CHECK: only its address is recorded; it must belong to the market program.
    #[account(owner = market_program.key() @ VaultError::WrongMarket)]
    pub market_reserve: UncheckedAccount<'info>,

    #[account(
        init,
        payer = payer,
        space = VaultConfig::DISCRIMINATOR.len() + VaultConfig::INIT_SPACE,
        seeds = [
            CONFIG_SEED,
            market_program.key().as_ref(),
            market_reserve.key().as_ref(),
            base_mint.key().as_ref(),
            receipt_mint.key().as_ref(),
        ],
        bump
    )]
    pub config: Box<Account<'info, VaultConfig>>,

    /// CHECK: PDA that owns the vault's receipts and is the share mint's authority. Holds no data.
    #[account(seeds = [AUTHORITY_SEED, config.key().as_ref()], bump)]
    pub vault_authority: UncheckedAccount<'info>,

    /// The vault's own share token. No freeze authority, so shares can never be frozen.
    #[account(
        init,
        payer = payer,
        seeds = [SHARES_SEED, config.key().as_ref()],
        bump,
        mint::decimals = receipt_mint.decimals.saturating_add(DECIMALS_OFFSET),
        mint::authority = vault_authority,
    )]
    pub share_mint: Box<Account<'info, Mint>>,

    /// The pooled market position, held as receipt tokens.
    #[account(
        init,
        payer = payer,
        associated_token::mint = receipt_mint,
        associated_token::authority = vault_authority,
    )]
    pub vault_receipts: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

impl<'info> InitializeVault<'info> {
    pub fn initialize_vault(&mut self, bumps: &InitializeVaultBumps) -> Result<()> {
        // A freezable receipt mint would let a third party block every exit.
        require!(
            self.receipt_mint.freeze_authority.is_none(),
            VaultError::FreezableReceiptMint
        );

        self.config.set_inner(VaultConfig {
            base_mint: self.base_mint.key(),
            receipt_mint: self.receipt_mint.key(),
            share_mint: self.share_mint.key(),
            vault_receipts: self.vault_receipts.key(),
            market_program: self.market_program.key(),
            market_reserve: self.market_reserve.key(),
            authority_bump: bumps.vault_authority,
            config_bump: bumps.config,
        });

        Ok(())
    }
}
