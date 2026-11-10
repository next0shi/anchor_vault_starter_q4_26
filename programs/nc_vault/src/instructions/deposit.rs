use crate::{
    constants::AUTHORITY_SEED,
    error::VaultError,
    math::shares_for_receipts,
    state::VaultConfig,
};
use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, MintTo, Token, TokenAccount};
use mock_market::program::MockMarket;

#[derive(Accounts)]
pub struct Deposit<'info> {
    pub user: Signer<'info>,

    pub config: Box<Account<'info, VaultConfig>>,

    /// CHECK: PDA, only used as a signer and token owner
    #[account(seeds = [AUTHORITY_SEED, config.key().as_ref()], bump = config.authority_bump)]
    pub vault_authority: UncheckedAccount<'info>,

    #[account(mut, address = config.share_mint)]
    pub share_mint: Box<Account<'info, Mint>>,

    #[account(mut, address = config.vault_receipts)]
    pub vault_receipts: Box<Account<'info, TokenAccount>>,

    #[account(mut, token::mint = config.base_mint, token::authority = user)]
    pub user_base: Box<Account<'info, TokenAccount>>,

    #[account(mut, token::mint = share_mint, token::authority = user)]
    pub user_shares: Box<Account<'info, TokenAccount>>,

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

impl<'info> Deposit<'info> {
    pub fn deposit(&mut self, amount: u64) -> Result<()> {
        require!(amount > 0, VaultError::ZeroAmount);

        let total_shares = self.share_mint.supply;
        let receipts_before = self.vault_receipts.amount;

        // 1. Put the user's base asset to work. The user signs the transfer into the market
        //    and the market sends the receipts straight to the vault, so the vault never
        //    holds base asset.
        mock_market::cpi::deposit_liquidity(
            CpiContext::new(
                self.market_program.key(),
                mock_market::cpi::accounts::DepositLiquidity {
                    depositor: self.user.to_account_info(),
                    reserve: self.market_reserve.to_account_info(),
                    source_liquidity: self.user_base.to_account_info(),
                    liquidity_vault: self.market_liquidity.to_account_info(),
                    receipt_mint: self.receipt_mint.to_account_info(),
                    destination_receipts: self.vault_receipts.to_account_info(),
                    reserve_authority: self.market_authority.to_account_info(),
                    token_program: self.token_program.to_account_info(),
                },
            ),
            amount,
        )?;

        // 2. Measure what actually arrived instead of trusting any return value.
        self.vault_receipts.reload()?;
        let received = self
            .vault_receipts
            .amount
            .checked_sub(receipts_before)
            .ok_or(VaultError::MathOverflow)?;
        require!(received > 0, VaultError::NothingReceived);

        // 3. Mint shares pro-rata to what the vault held BEFORE this deposit.
        let shares = shares_for_receipts(received, total_shares, receipts_before)?;
        require!(shares > 0, VaultError::DustAmount);

        let config_key = self.config.key();
        let seeds: &[&[u8]] = &[
            AUTHORITY_SEED,
            config_key.as_ref(),
            &[self.config.authority_bump],
        ];
        token::mint_to(
            CpiContext::new_with_signer(
                self.token_program.key(),
                MintTo {
                    mint: self.share_mint.to_account_info(),
                    to: self.user_shares.to_account_info(),
                    authority: self.vault_authority.to_account_info(),
                },
                &[seeds],
            ),
            shares,
        )
    }
}
