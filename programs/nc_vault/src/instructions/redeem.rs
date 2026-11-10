use crate::{
    constants::AUTHORITY_SEED,
    error::VaultError,
    math::receipts_for_shares,
    state::VaultConfig,
};
use anchor_lang::prelude::*;
use anchor_spl::token::{self, Burn, Mint, Token, TokenAccount};
use mock_market::program::MockMarket;

/// Exit to the BASE asset. This path calls the market, so it only works while the market
/// has liquidity. If it doesn't, use `redeem_in_kind`.
#[derive(Accounts)]
pub struct Redeem<'info> {
    pub user: Signer<'info>,

    pub config: Box<Account<'info, VaultConfig>>,

    /// CHECK: PDA, signs for the vault's receipts
    #[account(seeds = [AUTHORITY_SEED, config.key().as_ref()], bump = config.authority_bump)]
    pub vault_authority: UncheckedAccount<'info>,

    #[account(mut, address = config.share_mint)]
    pub share_mint: Box<Account<'info, Mint>>,

    #[account(mut, address = config.vault_receipts)]
    pub vault_receipts: Box<Account<'info, TokenAccount>>,

    #[account(mut, token::mint = share_mint, token::authority = user)]
    pub user_shares: Box<Account<'info, TokenAccount>>,

    #[account(mut, token::mint = config.base_mint)]
    pub destination_base: Box<Account<'info, TokenAccount>>,

    // ---- market side: pinned to the addresses stored in `config` ----
    #[account(address = config.market_program)]
    pub market_program: Program<'info, MockMarket>,

    /// CHECK: pinned to the reserve stored in `config`
    #[account(address = config.market_reserve)]
    pub market_reserve: UncheckedAccount<'info>,

    /// CHECK: checked by the market against its own reserve
    #[account(mut)]
    pub market_liquidity: UncheckedAccount<'info>,

    /// CHECK: checked by the market against its own reserve
    pub market_authority: UncheckedAccount<'info>,

    #[account(mut, address = config.receipt_mint)]
    pub receipt_mint: Box<Account<'info, Mint>>,

    pub token_program: Program<'info, Token>,
}

impl<'info> Redeem<'info> {
    pub fn redeem(&mut self, shares: u64, min_base_out: u64) -> Result<()> {
        require!(shares > 0, VaultError::ZeroAmount);

        let receipts_out =
            receipts_for_shares(shares, self.share_mint.supply, self.vault_receipts.amount)?;
        require!(receipts_out > 0, VaultError::DustAmount);

        // 1. Burn the user's shares (the user signs).
        token::burn(
            CpiContext::new(
                self.token_program.key(),
                Burn {
                    mint: self.share_mint.to_account_info(),
                    from: self.user_shares.to_account_info(),
                    authority: self.user.to_account_info(),
                },
            ),
            shares,
        )?;

        // 2. Hand the matching receipts to the market. The vault authority PDA signs as
        //    their owner, and the base asset goes straight to the destination.
        let base_before = self.destination_base.amount;
        let config_key = self.config.key();
        let seeds: &[&[u8]] = &[
            AUTHORITY_SEED,
            config_key.as_ref(),
            &[self.config.authority_bump],
        ];
        mock_market::cpi::redeem_receipts(
            CpiContext::new_with_signer(
                self.market_program.key(),
                mock_market::cpi::accounts::RedeemReceipts {
                    owner: self.vault_authority.to_account_info(),
                    reserve: self.market_reserve.to_account_info(),
                    receipt_mint: self.receipt_mint.to_account_info(),
                    source_receipts: self.vault_receipts.to_account_info(),
                    liquidity_vault: self.market_liquidity.to_account_info(),
                    destination_liquidity: self.destination_base.to_account_info(),
                    reserve_authority: self.market_authority.to_account_info(),
                    token_program: self.token_program.to_account_info(),
                },
                &[seeds],
            ),
            receipts_out,
        )?;

        // 3. Slippage check on what actually arrived.
        self.destination_base.reload()?;
        let received = self
            .destination_base
            .amount
            .checked_sub(base_before)
            .ok_or(VaultError::MathOverflow)?;
        require!(received >= min_base_out, VaultError::SlippageExceeded);

        Ok(())
    }
}
