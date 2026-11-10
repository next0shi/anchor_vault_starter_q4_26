pub mod constants;
pub mod error;
pub mod instructions;
pub mod math;
pub mod state;

use anchor_lang::prelude::*;

pub use constants::*;
pub use instructions::*;
pub use state::*;

declare_id!("EUBEXsL7gF5o7nuLbB3EgpmCR284iTBNqo1UERFYkwzp");

#[program]
pub mod nc_vault {
    use super::*;

    pub fn initialize_vault(ctx: Context<InitializeVault>) -> Result<()> {
        ctx.accounts.initialize_vault(&ctx.bumps)
    }

    pub fn deposit(ctx: Context<Deposit>, amount: u64) -> Result<()> {
        ctx.accounts.deposit(amount)
    }

    /// Exit to the base asset. Needs liquidity in the market.
    pub fn redeem(ctx: Context<Redeem>, shares: u64, min_base_out: u64) -> Result<()> {
        ctx.accounts.redeem(shares, min_base_out)
    }

    /// Exit to the market position itself. Needs nothing from the market.
    pub fn redeem_in_kind(ctx: Context<RedeemInKind>, shares: u64) -> Result<()> {
        ctx.accounts.redeem_in_kind(shares)
    }
}
