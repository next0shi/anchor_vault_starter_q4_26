//! LiteSVM tests for the non-custodial vault and its in-kind exit.
//!
//! Build both programs first (`anchor build`): the .so files are embedded at compile time.

use {
    anchor_lang::{
        prelude::Pubkey,
        solana_program::{instruction::Instruction, system_program},
        AccountDeserialize, InstructionData, ToAccountMetas,
    },
    anchor_spl::{associated_token, token},
    litesvm::{
        types::{TransactionMetadata, TransactionResult},
        LiteSVM,
    },
    litesvm_token::{get_spl_account, spl_token, CreateAssociatedTokenAccount, CreateMint, MintTo},
    mock_market::{
        AUTHORITY_SEED as MARKET_AUTHORITY_SEED, LIQUIDITY_SEED, RECEIPT_MINT_SEED, RESERVE_SEED,
    },
    nc_vault::{math, VaultConfig, AUTHORITY_SEED, CONFIG_SEED, DECIMALS_OFFSET, SHARES_SEED},
    solana_keypair::Keypair,
    solana_message::{Message, VersionedMessage},
    solana_signer::Signer,
    solana_transaction::versioned::VersionedTransaction,
};

const BASE_DECIMALS: u8 = 6;
const ONE: u64 = 1_000_000; // 1.0 of a 6-decimal token
const DEPOSIT: u64 = 1_000 * ONE;

// ---------- helpers ----------

/// Sends one instruction, signed (and paid for) by `payer`.
fn send(svm: &mut LiteSVM, payer: &Keypair, ix: Instruction) -> TransactionResult {
    let blockhash = svm.latest_blockhash();
    let msg = Message::new_with_blockhash(&[ix], Some(&payer.pubkey()), &blockhash);
    let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(msg), &[payer]).unwrap();
    let result = svm.send_transaction(tx);
    // LiteSVM rejects a transaction it has already seen, so move to a fresh blockhash.
    svm.expire_blockhash();
    result
}

/// Happy path: unwrap, but show the program logs if it failed.
fn ok(result: TransactionResult) -> TransactionMetadata {
    result.unwrap_or_else(|e| panic!("tx failed: {:?}\nlogs: {:#?}", e.err, e.meta.logs))
}

/// Sad path: the tx must fail, and the logs must mention `expected`.
fn assert_fails_with(result: TransactionResult, expected: &str) {
    let e = result.expect_err("transaction should have failed");
    let logs = e.meta.logs.join("\n");
    assert!(logs.contains(expected), "expected {expected:?} in logs:\n{logs}");
}

fn pda(seeds: &[&[u8]], program: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(seeds, program).0
}

fn ata(owner: &Pubkey, mint: &Pubkey) -> Pubkey {
    pda(
        &[owner.as_ref(), token::ID.as_ref(), mint.as_ref()],
        &associated_token::ID,
    )
}

struct Market {
    reserve: Pubkey,
    authority: Pubkey,
    liquidity: Pubkey,
    receipt_mint: Pubkey,
}

struct Vault {
    config: Pubkey,
    authority: Pubkey,
    share_mint: Pubkey,
    receipts: Pubkey,
}

struct User {
    kp: Keypair,
    base: Pubkey,
    shares: Pubkey,
    receipts: Pubkey,
}

struct Env {
    svm: LiteSVM,
    payer: Keypair,
    base_mint: Pubkey,
    market: Market,
    vault: Vault,
}

fn vault_addrs(market: &Market, base_mint: &Pubkey, receipt_mint: &Pubkey) -> Vault {
    let program = nc_vault::id();
    let config = pda(
        &[
            CONFIG_SEED,
            mock_market::id().as_ref(),
            market.reserve.as_ref(),
            base_mint.as_ref(),
            receipt_mint.as_ref(),
        ],
        &program,
    );
    let authority = pda(&[AUTHORITY_SEED, config.as_ref()], &program);
    let share_mint = pda(&[SHARES_SEED, config.as_ref()], &program);
    let receipts = ata(&authority, receipt_mint);
    Vault { config, authority, share_mint, receipts }
}

fn initialize_vault_ix(
    payer: &Pubkey,
    base_mint: &Pubkey,
    receipt_mint: &Pubkey,
    market: &Market,
    vault: &Vault,
) -> Instruction {
    Instruction::new_with_bytes(
        nc_vault::id(),
        &nc_vault::instruction::InitializeVault {}.data(),
        nc_vault::accounts::InitializeVault {
            payer: *payer,
            base_mint: *base_mint,
            receipt_mint: *receipt_mint,
            market_program: mock_market::id(),
            market_reserve: market.reserve,
            config: vault.config,
            vault_authority: vault.authority,
            share_mint: vault.share_mint,
            vault_receipts: vault.receipts,
            token_program: token::ID,
            associated_token_program: associated_token::ID,
            system_program: system_program::ID,
        }
        .to_account_metas(None),
    )
}

/// A fresh SVM with both programs loaded, a base token, a market reserve and a vault on top of it.
fn setup() -> Env {
    let mut svm = LiteSVM::new();
    // Embedded at compile time, so run `anchor build` first.
    svm.add_program(
        nc_vault::id(),
        include_bytes!(concat!(env!("CARGO_TARGET_TMPDIR"), "/../deploy/nc_vault.so")),
    )
    .unwrap();
    svm.add_program(
        mock_market::id(),
        include_bytes!(concat!(env!("CARGO_TARGET_TMPDIR"), "/../deploy/mock_market.so")),
    )
    .unwrap();

    let payer = Keypair::new();
    svm.airdrop(&payer.pubkey(), 1_000_000_000_000).unwrap();

    let base_mint = CreateMint::new(&mut svm, &payer)
        .decimals(BASE_DECIMALS)
        .send()
        .unwrap();

    // The market: a pool for the base token, with its own receipt token.
    let market_program = mock_market::id();
    let reserve = pda(&[RESERVE_SEED, base_mint.as_ref()], &market_program);
    let market = Market {
        reserve,
        authority: pda(&[MARKET_AUTHORITY_SEED, reserve.as_ref()], &market_program),
        liquidity: pda(&[LIQUIDITY_SEED, reserve.as_ref()], &market_program),
        receipt_mint: pda(&[RECEIPT_MINT_SEED, reserve.as_ref()], &market_program),
    };
    let ix = Instruction::new_with_bytes(
        market_program,
        &mock_market::instruction::InitReserve {}.data(),
        mock_market::accounts::InitReserve {
            payer: payer.pubkey(),
            base_mint,
            reserve: market.reserve,
            reserve_authority: market.authority,
            receipt_mint: market.receipt_mint,
            liquidity_vault: market.liquidity,
            token_program: token::ID,
            system_program: system_program::ID,
        }
        .to_account_metas(None),
    );
    ok(send(&mut svm, &payer, ix));

    // The vault on top of that market.
    let vault = vault_addrs(&market, &base_mint, &market.receipt_mint);
    let ix = initialize_vault_ix(&payer.pubkey(), &base_mint, &market.receipt_mint, &market, &vault);
    ok(send(&mut svm, &payer, ix));

    Env { svm, payer, base_mint, market, vault }
}

impl Env {
    /// A funded user with token accounts for base asset, vault shares and market receipts.
    fn new_user(&mut self, base_amount: u64) -> User {
        let kp = Keypair::new();
        let pk = kp.pubkey();
        self.svm.airdrop(&pk, 10_000_000_000).unwrap();

        let base = CreateAssociatedTokenAccount::new(&mut self.svm, &self.payer, &self.base_mint)
            .owner(&pk)
            .send()
            .unwrap();
        let shares =
            CreateAssociatedTokenAccount::new(&mut self.svm, &self.payer, &self.vault.share_mint)
                .owner(&pk)
                .send()
                .unwrap();
        let receipts = CreateAssociatedTokenAccount::new(
            &mut self.svm,
            &self.payer,
            &self.market.receipt_mint,
        )
        .owner(&pk)
        .send()
        .unwrap();

        if base_amount > 0 {
            MintTo::new(&mut self.svm, &self.payer, &self.base_mint, &base, base_amount)
                .send()
                .unwrap();
        }
        User { kp, base, shares, receipts }
    }

    fn balance(&self, account: &Pubkey) -> u64 {
        get_spl_account::<spl_token::state::Account>(&self.svm, account)
            .unwrap()
            .amount
    }

    fn supply(&self, mint: &Pubkey) -> u64 {
        get_spl_account::<spl_token::state::Mint>(&self.svm, mint)
            .unwrap()
            .supply
    }

    // ----- vault instructions -----

    fn deposit_ix(&self, user: &User, amount: u64) -> Instruction {
        Instruction::new_with_bytes(
            nc_vault::id(),
            &nc_vault::instruction::Deposit { amount }.data(),
            nc_vault::accounts::Deposit {
                user: user.kp.pubkey(),
                config: self.vault.config,
                vault_authority: self.vault.authority,
                share_mint: self.vault.share_mint,
                vault_receipts: self.vault.receipts,
                user_base: user.base,
                user_shares: user.shares,
                market_program: mock_market::id(),
                market_reserve: self.market.reserve,
                market_liquidity: self.market.liquidity,
                market_authority: self.market.authority,
                receipt_mint: self.market.receipt_mint,
                token_program: token::ID,
            }
            .to_account_metas(None),
        )
    }

    fn redeem_ix(&self, user: &User, shares: u64, min_base_out: u64) -> Instruction {
        Instruction::new_with_bytes(
            nc_vault::id(),
            &nc_vault::instruction::Redeem { shares, min_base_out }.data(),
            nc_vault::accounts::Redeem {
                user: user.kp.pubkey(),
                config: self.vault.config,
                vault_authority: self.vault.authority,
                share_mint: self.vault.share_mint,
                vault_receipts: self.vault.receipts,
                user_shares: user.shares,
                destination_base: user.base,
                market_program: mock_market::id(),
                market_reserve: self.market.reserve,
                market_liquidity: self.market.liquidity,
                market_authority: self.market.authority,
                receipt_mint: self.market.receipt_mint,
                token_program: token::ID,
            }
            .to_account_metas(None),
        )
    }

    fn redeem_in_kind_ix(
        &self,
        signer: &Pubkey,
        user_shares: &Pubkey,
        destination_receipts: &Pubkey,
        shares: u64,
    ) -> Instruction {
        Instruction::new_with_bytes(
            nc_vault::id(),
            &nc_vault::instruction::RedeemInKind { shares }.data(),
            nc_vault::accounts::RedeemInKind {
                user: *signer,
                config: self.vault.config,
                vault_authority: self.vault.authority,
                share_mint: self.vault.share_mint,
                vault_receipts: self.vault.receipts,
                user_shares: *user_shares,
                destination_receipts: *destination_receipts,
                token_program: token::ID,
            }
            .to_account_metas(None),
        )
    }

    fn deposit(&mut self, user: &User, amount: u64) -> TransactionResult {
        let ix = self.deposit_ix(user, amount);
        send(&mut self.svm, &user.kp, ix)
    }

    fn redeem(&mut self, user: &User, shares: u64, min_base_out: u64) -> TransactionResult {
        let ix = self.redeem_ix(user, shares, min_base_out);
        send(&mut self.svm, &user.kp, ix)
    }

    fn redeem_in_kind(&mut self, user: &User, shares: u64) -> TransactionResult {
        let ix = self.redeem_in_kind_ix(&user.kp.pubkey(), &user.shares, &user.receipts, shares);
        send(&mut self.svm, &user.kp, ix)
    }

    // ----- talking to the market directly -----

    /// Deposits into the market and sends the receipts to any account.
    fn market_deposit(
        &mut self,
        depositor: &User,
        destination_receipts: &Pubkey,
        amount: u64,
    ) -> TransactionResult {
        let ix = Instruction::new_with_bytes(
            mock_market::id(),
            &mock_market::instruction::DepositLiquidity { amount }.data(),
            mock_market::accounts::DepositLiquidity {
                depositor: depositor.kp.pubkey(),
                reserve: self.market.reserve,
                source_liquidity: depositor.base,
                liquidity_vault: self.market.liquidity,
                receipt_mint: self.market.receipt_mint,
                destination_receipts: *destination_receipts,
                reserve_authority: self.market.authority,
                token_program: token::ID,
            }
            .to_account_metas(None),
        );
        send(&mut self.svm, &depositor.kp, ix)
    }

    /// Simulates 100% utilization: borrowers take every last bit of cash out of the pool.
    fn drain_market_liquidity(&mut self) {
        let amount = self.balance(&self.market.liquidity);
        let borrower =
            CreateAssociatedTokenAccount::new(&mut self.svm, &self.payer, &self.base_mint)
                .owner(&self.payer.pubkey())
                .send()
                .unwrap();
        let ix = Instruction::new_with_bytes(
            mock_market::id(),
            &mock_market::instruction::Borrow { amount }.data(),
            mock_market::accounts::Borrow {
                reserve: self.market.reserve,
                liquidity_vault: self.market.liquidity,
                borrower_account: borrower,
                reserve_authority: self.market.authority,
                token_program: token::ID,
            }
            .to_account_metas(None),
        );
        ok(send(&mut self.svm, &self.payer, ix));
    }

    /// Simulates interest: every receipt becomes worth more base asset.
    fn market_accrue_interest(&mut self, amount: u64) -> TransactionResult {
        let ix = Instruction::new_with_bytes(
            mock_market::id(),
            &mock_market::instruction::AccrueInterest { amount }.data(),
            mock_market::accounts::AccrueInterest { reserve: self.market.reserve }
                .to_account_metas(None),
        );
        send(&mut self.svm, &self.payer, ix)
    }
}

// ---------- tests ----------

#[test]
fn initialize_vault_pins_everything_and_has_no_admin() {
    let env = setup();

    let data = env.svm.get_account(&env.vault.config).expect("config exists").data;
    let config = VaultConfig::try_deserialize(&mut data.as_slice()).unwrap();
    assert_eq!(config.base_mint, env.base_mint);
    assert_eq!(config.receipt_mint, env.market.receipt_mint);
    assert_eq!(config.share_mint, env.vault.share_mint);
    assert_eq!(config.vault_receipts, env.vault.receipts);
    assert_eq!(config.market_program, mock_market::id());
    assert_eq!(config.market_reserve, env.market.reserve);

    // The share token: only the vault PDA can mint it, nobody can freeze it.
    let shares =
        get_spl_account::<spl_token::state::Mint>(&env.svm, &env.vault.share_mint).unwrap();
    assert_eq!(shares.mint_authority.unwrap(), env.vault.authority);
    assert!(shares.freeze_authority.is_none());
    assert_eq!(shares.decimals, BASE_DECIMALS + DECIMALS_OFFSET);
    assert_eq!(shares.supply, 0);
}

#[test]
fn initialize_vault_rejects_a_freezable_receipt_mint() {
    let mut env = setup();

    let freezer = Keypair::new();
    let freezable = CreateMint::new(&mut env.svm, &env.payer)
        .decimals(BASE_DECIMALS)
        .freeze_authority(&freezer.pubkey())
        .send()
        .unwrap();

    let vault = vault_addrs(&env.market, &env.base_mint, &freezable);
    let ix = initialize_vault_ix(
        &env.payer.pubkey(),
        &env.base_mint,
        &freezable,
        &env.market,
        &vault,
    );
    assert_fails_with(send(&mut env.svm, &env.payer, ix), "FreezableReceiptMint");
}

#[test]
fn deposit_puts_funds_in_the_market_and_mints_shares() {
    let mut env = setup();
    let alice = env.new_user(DEPOSIT);

    ok(env.deposit(&alice, DEPOSIT));

    assert_eq!(env.balance(&alice.base), 0, "base asset left the user...");
    assert_eq!(env.balance(&env.market.liquidity), DEPOSIT, "...and sits in the market");
    assert_eq!(env.balance(&env.vault.receipts), DEPOSIT, "the vault holds the receipts");
    assert_eq!(env.balance(&alice.shares), DEPOSIT * 1_000);
    assert_eq!(env.supply(&env.vault.share_mint), DEPOSIT * 1_000);
}

#[test]
fn second_depositor_gets_shares_pro_rata() {
    let mut env = setup();
    let alice = env.new_user(DEPOSIT);
    let bob = env.new_user(500 * ONE);

    ok(env.deposit(&alice, DEPOSIT));
    ok(env.deposit(&bob, 500 * ONE));

    assert_eq!(env.balance(&bob.shares), 500 * ONE * 1_000);
    assert_eq!(env.balance(&env.vault.receipts), 1_500 * ONE);
    assert_eq!(env.supply(&env.vault.share_mint), 1_500 * ONE * 1_000);
}

#[test]
fn redeem_in_kind_returns_the_matching_receipts() {
    let mut env = setup();
    let alice = env.new_user(DEPOSIT);
    ok(env.deposit(&alice, DEPOSIT));
    let half = env.balance(&alice.shares) / 2;

    ok(env.redeem_in_kind(&alice, half));

    assert_eq!(env.balance(&alice.receipts), DEPOSIT / 2);
    assert_eq!(env.balance(&env.vault.receipts), DEPOSIT / 2);
    assert_eq!(env.balance(&alice.shares), half);
    assert_eq!(env.balance(&alice.base), 0, "no base asset moved");
}

#[test]
fn redeem_to_base_asset_works_while_the_market_has_cash() {
    let mut env = setup();
    let alice = env.new_user(DEPOSIT);
    ok(env.deposit(&alice, DEPOSIT));
    let shares = env.balance(&alice.shares);

    ok(env.redeem(&alice, shares, DEPOSIT));

    assert_eq!(env.balance(&alice.base), DEPOSIT);
    assert_eq!(env.balance(&env.vault.receipts), 0);
    assert_eq!(env.supply(&env.vault.share_mint), 0);
}

#[test]
fn in_kind_exit_works_when_the_market_has_zero_liquidity() {
    let mut env = setup();
    let alice = env.new_user(DEPOSIT);
    ok(env.deposit(&alice, DEPOSIT));
    let shares = env.balance(&alice.shares);

    env.drain_market_liquidity();
    assert_eq!(env.balance(&env.market.liquidity), 0);

    // Exit to the base asset: the market has no cash, so this fails...
    assert_fails_with(env.redeem(&alice, shares, 0), "InsufficientLiquidity");
    assert_eq!(env.balance(&alice.shares), shares, "a failed exit changes nothing");

    // ...but exiting in kind needs nothing from the market.
    ok(env.redeem_in_kind(&alice, shares));
    assert_eq!(env.balance(&alice.receipts), DEPOSIT);
    assert_eq!(env.balance(&env.vault.receipts), 0);
    assert_eq!(env.supply(&env.vault.share_mint), 0);
}

#[test]
fn the_in_kind_exit_touches_no_market_accounts() {
    let mut env = setup();
    let alice = env.new_user(DEPOSIT);
    ok(env.deposit(&alice, DEPOSIT));

    let ix = env.redeem_in_kind_ix(&alice.kp.pubkey(), &alice.shares, &alice.receipts, 1_000);
    let market_keys = [
        mock_market::id(),
        env.market.reserve,
        env.market.liquidity,
        env.market.authority,
        env.market.receipt_mint,
    ];
    for meta in &ix.accounts {
        assert!(
            !market_keys.contains(&meta.pubkey),
            "{} is a market account",
            meta.pubkey
        );
    }
}

#[test]
fn cannot_redeem_more_shares_than_owned() {
    let mut env = setup();
    let alice = env.new_user(DEPOSIT);
    ok(env.deposit(&alice, DEPOSIT));
    let shares = env.balance(&alice.shares);

    assert_fails_with(env.redeem_in_kind(&alice, shares + 1), "insufficient funds");
}

#[test]
fn only_the_share_owner_can_redeem() {
    let mut env = setup();
    let alice = env.new_user(DEPOSIT);
    let mallory = env.new_user(0);
    ok(env.deposit(&alice, DEPOSIT));

    // Mallory signs, but points at Alice's shares.
    let ix = env.redeem_in_kind_ix(
        &mallory.kp.pubkey(),
        &alice.shares,
        &mallory.receipts,
        1_000_000,
    );
    assert_fails_with(send(&mut env.svm, &mallory.kp, ix), "ConstraintTokenOwner");
    assert_eq!(env.balance(&env.vault.receipts), DEPOSIT, "vault untouched");
}

#[test]
fn zero_amounts_are_rejected() {
    let mut env = setup();
    let alice = env.new_user(DEPOSIT);
    ok(env.deposit(&alice, DEPOSIT));

    assert_fails_with(env.deposit(&alice, 0), "ZeroAmount");
    assert_fails_with(env.redeem_in_kind(&alice, 0), "ZeroAmount");
    assert_fails_with(env.redeem(&alice, 0, 0), "ZeroAmount");
}

#[test]
fn receipts_donated_to_the_vault_are_shared_pro_rata() {
    let mut env = setup();
    let alice = env.new_user(DEPOSIT);
    let donor = env.new_user(100 * ONE);
    ok(env.deposit(&alice, DEPOSIT));

    // The donor buys receipts from the market and sends them straight to the vault.
    let vault_receipts = env.vault.receipts;
    ok(env.market_deposit(&donor, &vault_receipts, 100 * ONE));
    assert_eq!(env.balance(&env.vault.receipts), DEPOSIT + 100 * ONE);

    let shares = env.balance(&alice.shares);
    ok(env.redeem_in_kind(&alice, shares));

    let expected = math::receipts_for_shares(shares, shares, DEPOSIT + 100 * ONE).unwrap();
    assert_eq!(env.balance(&alice.receipts), expected);
    assert!(expected > DEPOSIT, "the donation went to the only shareholder");
    assert!(expected >= DEPOSIT + 100 * ONE - 2, "at most a hair of dust is left behind");
}

#[test]
fn inflation_attack_does_not_steal_a_deposit() {
    let mut env = setup();
    let attacker = env.new_user(1 + 1_000 * ONE); // 1 unit to deposit, 1000 tokens to donate
    let victim = env.new_user(500 * ONE);

    // 1. The attacker becomes the first depositor with the smallest possible deposit.
    ok(env.deposit(&attacker, 1));
    // 2. They donate 1000 tokens' worth of receipts straight to the vault to inflate the price.
    let vault_receipts = env.vault.receipts;
    ok(env.market_deposit(&attacker, &vault_receipts, 1_000 * ONE));
    // 3. The victim deposits.
    ok(env.deposit(&victim, 500 * ONE));
    assert!(
        env.balance(&victim.shares) > 0,
        "plain maths would have minted zero shares here"
    );

    // Everyone exits in kind.
    let victim_shares = env.balance(&victim.shares);
    let attacker_shares = env.balance(&attacker.shares);
    ok(env.redeem_in_kind(&victim, victim_shares));
    ok(env.redeem_in_kind(&attacker, attacker_shares));

    let victim_out = env.balance(&victim.receipts);
    let attacker_out = env.balance(&attacker.receipts);
    assert!(victim_out >= 500 * ONE / 1_000 * 998, "victim keeps at least 99.8%");
    assert!(attacker_out < 1 + 1_000 * ONE, "attacker ends up poorer");
}

#[test]
fn shares_are_priced_in_receipts_not_in_base_asset() {
    let mut env = setup();
    let alice = env.new_user(DEPOSIT);
    let bob = env.new_user(DEPOSIT);
    ok(env.deposit(&alice, DEPOSIT));

    // Interest accrues: every receipt is now worth twice as much base asset.
    ok(env.market_accrue_interest(DEPOSIT));

    // Bob deposits the same base amount but gets half as many receipts...
    ok(env.deposit(&bob, DEPOSIT));
    assert_eq!(env.balance(&env.vault.receipts), 1_500 * ONE);
    // ...and shares track RECEIPTS (1000 per unit), not base asset.
    assert_eq!(env.balance(&bob.shares), 500 * ONE * 1_000);

    // Alice's in-kind exit is exactly her receipts, whatever the exchange rate did.
    let shares = env.balance(&alice.shares);
    ok(env.redeem_in_kind(&alice, shares));
    assert_eq!(env.balance(&alice.receipts), DEPOSIT);
}

#[test]
fn redeem_respects_min_base_out() {
    let mut env = setup();
    let alice = env.new_user(DEPOSIT);
    ok(env.deposit(&alice, DEPOSIT));
    let shares = env.balance(&alice.shares);

    assert_fails_with(env.redeem(&alice, shares, DEPOSIT + 1), "SlippageExceeded");
    assert_eq!(env.balance(&alice.shares), shares, "a failed exit changes nothing");

    ok(env.redeem(&alice, shares, DEPOSIT));
}

#[test]
fn deposit_rejects_a_market_account_the_vault_was_not_created_for() {
    let mut env = setup();
    let alice = env.new_user(DEPOSIT);

    // Swap the reserve pinned in `config` for a different address.
    let mut ix = env.deposit_ix(&alice, DEPOSIT);
    let fake = Keypair::new().pubkey();
    for meta in ix.accounts.iter_mut() {
        if meta.pubkey == env.market.reserve {
            meta.pubkey = fake;
        }
    }

    assert_fails_with(send(&mut env.svm, &alice.kp, ix), "ConstraintAddress");
    assert_eq!(env.balance(&alice.base), DEPOSIT, "nothing moved");
}
