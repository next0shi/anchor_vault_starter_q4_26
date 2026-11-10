use anchor_lang::prelude::*;

#[error_code]
pub enum VaultError {
    #[msg("Amount must be greater than zero")]
    ZeroAmount,
    #[msg("The receipt mint has a freeze authority, so exits could be blocked")]
    FreezableReceiptMint,
    #[msg("Arithmetic overflow")]
    MathOverflow,
    #[msg("Amount is too small to be worth anything")]
    DustAmount,
    #[msg("Received less than the minimum requested")]
    SlippageExceeded,
    #[msg("The market returned no receipts")]
    NothingReceived,
    #[msg("The reserve is not owned by the market program")]
    WrongMarket,
    #[msg("Destination must not be the vault's own receipt account")]
    InvalidDestination,
}
