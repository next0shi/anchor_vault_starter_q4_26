use anchor_lang::prelude::*;

#[error_code]
pub enum ErrorCode {
    #[msg("Amount must be greater than zero")]
    InvalidAmount,

    #[msg("Withdrawal leaves account below rent-exempt minimum")]
    InsufficientFunds,
}
