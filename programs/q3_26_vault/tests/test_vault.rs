use {
    anchor_lang::{
        prelude::Pubkey,
        solana_program::{instruction::Instruction, system_program},
        InstructionData, ToAccountMetas,
    },
    litesvm::{
        types::{TransactionMetadata, TransactionResult},
        LiteSVM,
    },
    q3_26_vault::{VAULT_SEED, VAULT_STATE_SEED},
    solana_keypair::Keypair,
    solana_message::{Message, VersionedMessage},
    solana_signer::Signer,
    solana_transaction::versioned::VersionedTransaction,
};

const AIRDROP: u64 = 2_000_000_000;
const DEPOSIT: u64 = 500_000_000;
const WITHDRAW: u64 = 100_000_000;

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
    assert!(
        logs.contains(expected),
        "expected {expected:?} in logs:\n{logs}"
    );
}

struct Ctx {
    svm: LiteSVM,
    program_id: Pubkey,
    user: Keypair,
    vault_state: Pubkey,
    vault: Pubkey,
}

fn setup() -> Ctx {
    let program_id = q3_26_vault::id();
    let user = Keypair::new();
    let (vault_state, _) =
        Pubkey::find_program_address(&[VAULT_STATE_SEED, user.pubkey().as_ref()], &program_id);
    let (vault, _) =
        Pubkey::find_program_address(&[VAULT_SEED, user.pubkey().as_ref()], &program_id);

    let mut svm = LiteSVM::new();
    // Baked in at compile time, so run `anchor build` first.
    let bytes = include_bytes!(concat!(
        env!("CARGO_TARGET_TMPDIR"),
        "/../deploy/q3_26_vault.so"
    ));
    svm.add_program(program_id, bytes).unwrap();
    svm.airdrop(&user.pubkey(), AIRDROP).unwrap();

    Ctx {
        svm,
        program_id,
        user,
        vault_state,
        vault,
    }
}

impl Ctx {
    fn balance(&self, key: &Pubkey) -> u64 {
        self.svm.get_balance(key).unwrap_or(0)
    }

    fn initialize(&mut self) -> TransactionResult {
        let ix = Instruction::new_with_bytes(
            self.program_id,
            &q3_26_vault::instruction::Initialize {}.data(),
            q3_26_vault::accounts::Initialize {
                user: self.user.pubkey(),
                vault_state: self.vault_state,
                vault: self.vault,
                system_program: system_program::ID,
            }
            .to_account_metas(None),
        );
        send(&mut self.svm, &self.user, ix)
    }

    fn deposit(&mut self, amount: u64) -> TransactionResult {
        let ix = Instruction::new_with_bytes(
            self.program_id,
            &q3_26_vault::instruction::Deposit { amount }.data(),
            q3_26_vault::accounts::Deposit {
                user: self.user.pubkey(),
                vault_state: self.vault_state,
                vault: self.vault,
                system_program: system_program::ID,
            }
            .to_account_metas(None),
        );
        send(&mut self.svm, &self.user, ix)
    }

    fn withdraw(&mut self, amount: u64) -> TransactionResult {
        let ix = Instruction::new_with_bytes(
            self.program_id,
            &q3_26_vault::instruction::Withdraw { amount }.data(),
            q3_26_vault::accounts::Withdraw {
                user: self.user.pubkey(),
                vault_state: self.vault_state,
                vault: self.vault,
                system_program: system_program::ID,
            }
            .to_account_metas(None),
        );
        send(&mut self.svm, &self.user, ix)
    }

    fn close(&mut self) -> TransactionResult {
        let ix = Instruction::new_with_bytes(
            self.program_id,
            &q3_26_vault::instruction::Close {}.data(),
            q3_26_vault::accounts::Close {
                user: self.user.pubkey(),
                vault_state: self.vault_state,
                vault: self.vault,
                system_program: system_program::ID,
            }
            .to_account_metas(None),
        );
        send(&mut self.svm, &self.user, ix)
    }
}

// ---------- tests ----------

#[test]
fn initialize_creates_state_and_funds_vault() {
    let mut c = setup();
    ok(c.initialize());

    let state = c
        .svm
        .get_account(&c.vault_state)
        .expect("vault_state should exist");
    assert_eq!(
        state.owner, c.program_id,
        "vault_state belongs to our program"
    );
    assert_eq!(
        c.balance(&c.vault),
        c.svm.minimum_balance_for_rent_exemption(0),
        "vault starts out rent-exempt"
    );
}

#[test]
fn deposit_increases_vault_balance() {
    let mut c = setup();
    ok(c.initialize());

    let before = c.balance(&c.vault);
    ok(c.deposit(DEPOSIT));

    assert_eq!(c.balance(&c.vault), before + DEPOSIT);
}

#[test]
fn zero_amounts_are_rejected() {
    let mut c = setup();
    ok(c.initialize());
    ok(c.deposit(DEPOSIT));

    assert_fails_with(c.deposit(0), "InvalidAmount");
    assert_fails_with(c.withdraw(0), "InvalidAmount");
}

#[test]
fn withdraw_sends_lamports_to_user() {
    let mut c = setup();
    ok(c.initialize());
    ok(c.deposit(DEPOSIT));

    let vault_before = c.balance(&c.vault);
    let user_before = c.balance(&c.user.pubkey());

    let meta = ok(c.withdraw(WITHDRAW));

    assert_eq!(c.balance(&c.vault), vault_before - WITHDRAW);
    // The user receives the amount but pays the transaction fee.
    assert_eq!(
        c.balance(&c.user.pubkey()),
        user_before + WITHDRAW - meta.fee
    );
}

#[test]
fn withdraw_cannot_touch_the_rent_reserve() {
    let mut c = setup();
    ok(c.initialize());
    ok(c.deposit(DEPOSIT));

    // One lamport more than was deposited would dip into the rent-exempt minimum.
    assert_fails_with(c.withdraw(DEPOSIT + 1), "InsufficientFunds");

    // Taking out exactly what was deposited is fine; the reserve stays.
    ok(c.withdraw(DEPOSIT));
    assert_eq!(
        c.balance(&c.vault),
        c.svm.minimum_balance_for_rent_exemption(0)
    );
}

#[test]
fn other_users_cannot_withdraw() {
    let mut c = setup();
    ok(c.initialize());
    ok(c.deposit(DEPOSIT));

    let attacker = Keypair::new();
    c.svm.airdrop(&attacker.pubkey(), AIRDROP).unwrap();

    // The attacker signs, but points at the victim's PDAs.
    let ix = Instruction::new_with_bytes(
        c.program_id,
        &q3_26_vault::instruction::Withdraw { amount: WITHDRAW }.data(),
        q3_26_vault::accounts::Withdraw {
            user: attacker.pubkey(),
            vault_state: c.vault_state,
            vault: c.vault,
            system_program: system_program::ID,
        }
        .to_account_metas(None),
    );

    let vault_before = c.balance(&c.vault);
    assert_fails_with(send(&mut c.svm, &attacker, ix), "ConstraintSeeds");
    assert_eq!(c.balance(&c.vault), vault_before, "vault untouched");
}

#[test]
fn close_returns_everything_and_removes_state() {
    let mut c = setup();
    ok(c.initialize());
    ok(c.deposit(DEPOSIT));

    let user_before = c.balance(&c.user.pubkey());
    let vault_before = c.balance(&c.vault);
    let state_before = c.balance(&c.vault_state);

    let meta = ok(c.close());

    assert!(
        c.svm.get_account(&c.vault_state).is_none(),
        "vault_state should be closed"
    );
    assert_eq!(c.balance(&c.vault), 0, "vault should be empty");
    // The user gets the vault's lamports AND vault_state's rent, minus the fee.
    assert_eq!(
        c.balance(&c.user.pubkey()),
        user_before + vault_before + state_before - meta.fee
    );
}

#[test]
fn vault_is_unusable_after_close() {
    let mut c = setup();
    ok(c.initialize());
    ok(c.close());

    // vault_state no longer exists, so Anchor refuses to load it.
    assert_fails_with(c.deposit(DEPOSIT), "AccountNotInitialized");
}
