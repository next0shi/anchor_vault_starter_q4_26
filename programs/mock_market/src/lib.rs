//! A deliberately tiny lending market used ONLY to test the non-custodial vault.
//!
//! * `init_reserve`      create a pool for one base asset plus its receipt token
//! * `deposit_liquidity` base asset in, receipt tokens out (pro-rata to the pool)
//! * `redeem_receipts`   receipt tokens in, base asset out; fails if the pool has no cash
//! * `borrow`            moves cash out of the pool (simulates 100% utilization)
//! * `accrue_interest`   increases what borrowers owe, so a receipt is worth more base asset
//!
//! There is no access control on purpose. Never deploy this to a real cluster.

use anchor_lang::prelude::*;
use anchor_spl::token::{self, Burn, Mint, MintTo, Token, TokenAccount, Transfer};

declare_id!("7ux52LojKUM12KicQiCM4iLHCjmVKBnUhtVCBHtCXYQY");

pub const RESERVE_SEED: &[u8] = b"reserve";
pub const AUTHORITY_SEED: &[u8] = b"reserve_authority";
pub const RECEIPT_MINT_SEED: &[u8] = b"receipt_mint";
pub const LIQUIDITY_SEED: &[u8] = b"liquidity";

#[program]
pub mod mock_market {
    use super::*;

    pub fn init_reserve(ctx: Context<InitReserve>) -> Result<()> {
        ctx.accounts.reserve.set_inner(Reserve {
            base_mint: ctx.accounts.base_mint.key(),
            receipt_mint: ctx.accounts.receipt_mint.key(),
            liquidity_vault: ctx.accounts.liquidity_vault.key(),
            borrowed: 0,
            authority_bump: ctx.bumps.reserve_authority,
            bump: ctx.bumps.reserve,
        });
        Ok(())
    }

    /// Base asset goes in (the depositor signs); receipts are minted to any receipt account.
    pub fn deposit_liquidity(ctx: Context<DepositLiquidity>, amount: u64) -> Result<()> {
        require!(amount > 0, MarketError::ZeroAmount);

        let supply = ctx.accounts.receipt_mint.supply;
        let total_assets = ctx
            .accounts
            .reserve
            .total_assets(ctx.accounts.liquidity_vault.amount)?;
        let receipts_out = if supply == 0 || total_assets == 0 {
            amount
        } else {
            mul_div(amount, supply, total_assets)?
        };
        require!(receipts_out > 0, MarketError::ZeroAmount);

        token::transfer(
            CpiContext::new(
                ctx.accounts.token_program.key(),
                Transfer {
                    from: ctx.accounts.source_liquidity.to_account_info(),
                    to: ctx.accounts.liquidity_vault.to_account_info(),
                    authority: ctx.accounts.depositor.to_account_info(),
                },
            ),
            amount,
        )?;

        let reserve_key = ctx.accounts.reserve.key();
        let seeds: &[&[u8]] = &[
            AUTHORITY_SEED,
            reserve_key.as_ref(),
            &[ctx.accounts.reserve.authority_bump],
        ];
        token::mint_to(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.key(),
                MintTo {
                    mint: ctx.accounts.receipt_mint.to_account_info(),
                    to: ctx.accounts.destination_receipts.to_account_info(),
                    authority: ctx.accounts.reserve_authority.to_account_info(),
                },
                &[seeds],
            ),
            receipts_out,
        )
    }

    /// Receipts are burned (their owner signs); base asset is paid out only if the pool has cash.
    pub fn redeem_receipts(ctx: Context<RedeemReceipts>, amount: u64) -> Result<()> {
        require!(amount > 0, MarketError::ZeroAmount);

        let supply = ctx.accounts.receipt_mint.supply;
        let total_assets = ctx
            .accounts
            .reserve
            .total_assets(ctx.accounts.liquidity_vault.amount)?;
        let base_out = mul_div(amount, total_assets, supply)?;
        require!(base_out > 0, MarketError::ZeroAmount);
        // The point of this mock: valid receipts can still be unredeemable right now.
        require!(
            ctx.accounts.liquidity_vault.amount >= base_out,
            MarketError::InsufficientLiquidity
        );

        token::burn(
            CpiContext::new(
                ctx.accounts.token_program.key(),
                Burn {
                    mint: ctx.accounts.receipt_mint.to_account_info(),
                    from: ctx.accounts.source_receipts.to_account_info(),
                    authority: ctx.accounts.owner.to_account_info(),
                },
            ),
            amount,
        )?;

        let reserve_key = ctx.accounts.reserve.key();
        let seeds: &[&[u8]] = &[
            AUTHORITY_SEED,
            reserve_key.as_ref(),
            &[ctx.accounts.reserve.authority_bump],
        ];
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.key(),
                Transfer {
                    from: ctx.accounts.liquidity_vault.to_account_info(),
                    to: ctx.accounts.destination_liquidity.to_account_info(),
                    authority: ctx.accounts.reserve_authority.to_account_info(),
                },
                &[seeds],
            ),
            base_out,
        )
    }

    /// Simulates borrowers taking cash out of the pool.
    pub fn borrow(ctx: Context<Borrow>, amount: u64) -> Result<()> {
        require!(amount > 0, MarketError::ZeroAmount);
        require!(
            ctx.accounts.liquidity_vault.amount >= amount,
            MarketError::InsufficientLiquidity
        );

        let reserve_key = ctx.accounts.reserve.key();
        let seeds: &[&[u8]] = &[
            AUTHORITY_SEED,
            reserve_key.as_ref(),
            &[ctx.accounts.reserve.authority_bump],
        ];
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.key(),
                Transfer {
                    from: ctx.accounts.liquidity_vault.to_account_info(),
                    to: ctx.accounts.borrower_account.to_account_info(),
                    authority: ctx.accounts.reserve_authority.to_account_info(),
                },
                &[seeds],
            ),
            amount,
        )?;

        ctx.accounts.reserve.borrowed = ctx
            .accounts
            .reserve
            .borrowed
            .checked_add(amount)
            .ok_or_else(|| error!(MarketError::MathOverflow))?;
        Ok(())
    }

    /// Simulates interest: borrowers owe more, so every receipt is worth more base asset.
    pub fn accrue_interest(ctx: Context<AccrueInterest>, amount: u64) -> Result<()> {
        ctx.accounts.reserve.borrowed = ctx
            .accounts
            .reserve
            .borrowed
            .checked_add(amount)
            .ok_or_else(|| error!(MarketError::MathOverflow))?;
        Ok(())
    }
}

#[account]
#[derive(InitSpace)]
pub struct Reserve {
    pub base_mint: Pubkey,
    pub receipt_mint: Pubkey,
    pub liquidity_vault: Pubkey,
    /// Base asset borrowers owe (lent out plus accrued interest). Counts towards the pool's value.
    pub borrowed: u64,
    pub authority_bump: u8,
    pub bump: u8,
}

impl Reserve {
    /// Everything the pool is worth in base asset: cash on hand plus what borrowers owe.
    pub fn total_assets(&self, available: u64) -> Result<u64> {
        available
            .checked_add(self.borrowed)
            .ok_or_else(|| error!(MarketError::MathOverflow))
    }
}

fn mul_div(a: u64, b: u64, c: u64) -> Result<u64> {
    let v = (a as u128)
        .checked_mul(b as u128)
        .ok_or_else(|| error!(MarketError::MathOverflow))?
        .checked_div(c as u128)
        .ok_or_else(|| error!(MarketError::MathOverflow))?;
    u64::try_from(v).map_err(|_| error!(MarketError::MathOverflow))
}

#[derive(Accounts)]
pub struct InitReserve<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    pub base_mint: Box<Account<'info, Mint>>,

    #[account(
        init,
        payer = payer,
        space = Reserve::DISCRIMINATOR.len() + Reserve::INIT_SPACE,
        seeds = [RESERVE_SEED, base_mint.key().as_ref()],
        bump
    )]
    pub reserve: Box<Account<'info, Reserve>>,

    /// CHECK: PDA that owns the pool's cash and mints receipts. Holds no data.
    #[account(seeds = [AUTHORITY_SEED, reserve.key().as_ref()], bump)]
    pub reserve_authority: UncheckedAccount<'info>,

    #[account(
        init,
        payer = payer,
        seeds = [RECEIPT_MINT_SEED, reserve.key().as_ref()],
        bump,
        mint::decimals = base_mint.decimals,
        mint::authority = reserve_authority,
    )]
    pub receipt_mint: Box<Account<'info, Mint>>,

    #[account(
        init,
        payer = payer,
        seeds = [LIQUIDITY_SEED, reserve.key().as_ref()],
        bump,
        token::mint = base_mint,
        token::authority = reserve_authority,
    )]
    pub liquidity_vault: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct DepositLiquidity<'info> {
    pub depositor: Signer<'info>,

    #[account(seeds = [RESERVE_SEED, reserve.base_mint.as_ref()], bump = reserve.bump)]
    pub reserve: Box<Account<'info, Reserve>>,

    #[account(mut, token::mint = reserve.base_mint, token::authority = depositor)]
    pub source_liquidity: Box<Account<'info, TokenAccount>>,

    #[account(mut, address = reserve.liquidity_vault)]
    pub liquidity_vault: Box<Account<'info, TokenAccount>>,

    #[account(mut, address = reserve.receipt_mint)]
    pub receipt_mint: Box<Account<'info, Mint>>,

    #[account(mut, token::mint = receipt_mint)]
    pub destination_receipts: Box<Account<'info, TokenAccount>>,

    /// CHECK: PDA signer for the pool
    #[account(seeds = [AUTHORITY_SEED, reserve.key().as_ref()], bump = reserve.authority_bump)]
    pub reserve_authority: UncheckedAccount<'info>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct RedeemReceipts<'info> {
    pub owner: Signer<'info>,

    #[account(seeds = [RESERVE_SEED, reserve.base_mint.as_ref()], bump = reserve.bump)]
    pub reserve: Box<Account<'info, Reserve>>,

    #[account(mut, address = reserve.receipt_mint)]
    pub receipt_mint: Box<Account<'info, Mint>>,

    #[account(mut, token::mint = receipt_mint, token::authority = owner)]
    pub source_receipts: Box<Account<'info, TokenAccount>>,

    #[account(mut, address = reserve.liquidity_vault)]
    pub liquidity_vault: Box<Account<'info, TokenAccount>>,

    #[account(mut, token::mint = reserve.base_mint)]
    pub destination_liquidity: Box<Account<'info, TokenAccount>>,

    /// CHECK: PDA signer for the pool
    #[account(seeds = [AUTHORITY_SEED, reserve.key().as_ref()], bump = reserve.authority_bump)]
    pub reserve_authority: UncheckedAccount<'info>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct Borrow<'info> {
    #[account(mut, seeds = [RESERVE_SEED, reserve.base_mint.as_ref()], bump = reserve.bump)]
    pub reserve: Box<Account<'info, Reserve>>,

    #[account(mut, address = reserve.liquidity_vault)]
    pub liquidity_vault: Box<Account<'info, TokenAccount>>,

    #[account(mut, token::mint = reserve.base_mint)]
    pub borrower_account: Box<Account<'info, TokenAccount>>,

    /// CHECK: PDA signer for the pool
    #[account(seeds = [AUTHORITY_SEED, reserve.key().as_ref()], bump = reserve.authority_bump)]
    pub reserve_authority: UncheckedAccount<'info>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct AccrueInterest<'info> {
    #[account(mut, seeds = [RESERVE_SEED, reserve.base_mint.as_ref()], bump = reserve.bump)]
    pub reserve: Box<Account<'info, Reserve>>,
}

#[error_code]
pub enum MarketError {
    #[msg("Amount must be greater than zero")]
    ZeroAmount,
    #[msg("Arithmetic overflow")]
    MathOverflow,
    #[msg("The pool does not have enough available liquidity")]
    InsufficientLiquidity,
}
