use crate::{
    constants::{VAULT_SEED, VAULT_STATE_SEED},
    error::ErrorCode,
    state::VaultState,
};

use anchor_lang::{
    prelude::*,
    system_program::{transfer, Transfer},
};

#[derive(Accounts)]
pub struct Withdraw<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    #[account(
        seeds = [VAULT_STATE_SEED, user.key().as_ref()],
        bump = vault_state.state_bump,
    )]
    pub vault_state: Account<'info, VaultState>,

    #[account(
        mut,
        seeds = [VAULT_SEED, user.key().as_ref()],
        bump = vault_state.vault_bump
    )]
    pub vault: SystemAccount<'info>,

    pub system_program: Program<'info, System>,
}

impl<'info> Withdraw<'info> {
    pub fn withdraw(&mut self, amount: u64) -> Result<()> {
        require!(amount > 0, ErrorCode::InvalidAmount);

        let rent_exempt = Rent::get()?.minimum_balance(self.vault.data_len());

        require!(
            self.vault.lamports().saturating_sub(amount) >= rent_exempt,
            ErrorCode::InsufficientFunds
        );

        let cpi_program = self.system_program.key();

        let cpi_accounts = Transfer {
            from: self.vault.to_account_info(),
            to: self.user.to_account_info(),
        };

        let signer_seeds: [&[&[u8]]; 1] = [&[
            VAULT_SEED,
            self.user.key.as_ref(),
            &[self.vault_state.vault_bump],
        ]];

        let cpi_context = CpiContext::new_with_signer(cpi_program, cpi_accounts, &signer_seeds);

        transfer(cpi_context, amount)
    }
}
