use crate::{
    constants::AUTHORITY_SEED,
    error::VaultError,
    math::receipts_for_shares,
    state::VaultConfig,
};
use anchor_lang::prelude::*;
use anchor_spl::token::{self, Burn, Mint, Token, TokenAccount, Transfer};

/// Exit to the market POSITION itself (the receipt tokens).
///
/// Look at the accounts: there is not a single market account in this list. Nothing
/// outside the vault and the token program can make this instruction fail, which is
/// why it works with zero liquidity, a paused market, or a market program that is gone.
#[derive(Accounts)]
pub struct RedeemInKind<'info> {
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

    #[account(
        mut,
        token::mint = config.receipt_mint,
        constraint = destination_receipts.key() != vault_receipts.key() @ VaultError::InvalidDestination
    )]
    pub destination_receipts: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
}

impl<'info> RedeemInKind<'info> {
    pub fn redeem_in_kind(&mut self, shares: u64) -> Result<()> {
        require!(shares > 0, VaultError::ZeroAmount);

        // Pro-rata slice of everything the vault holds. No prices, no oracle, no market.
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

        // 2. Send the matching receipts to the user (the vault authority PDA signs).
        let config_key = self.config.key();
        let seeds: &[&[u8]] = &[
            AUTHORITY_SEED,
            config_key.as_ref(),
            &[self.config.authority_bump],
        ];
        token::transfer(
            CpiContext::new_with_signer(
                self.token_program.key(),
                Transfer {
                    from: self.vault_receipts.to_account_info(),
                    to: self.destination_receipts.to_account_info(),
                    authority: self.vault_authority.to_account_info(),
                },
                &[seeds],
            ),
            receipts_out,
        )
    }
}
