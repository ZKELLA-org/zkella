#![no_std]

use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short,
    Address, Bytes, BytesN, Env,
};

#[contracttype]
pub enum StorageKey {
    SwapState(BytesN<32>),
    ApprovedRelayer(Address),
    Admin,
}

#[contracttype]
#[derive(Clone, PartialEq)]
pub enum SwapStatus {
    Committed,
    Executed,
    Claimed,
    Cancelled,
}

#[contracttype]
pub struct SwapState {
    pub intent_commitment: BytesN<32>,
    pub nullifier_in:      BytesN<32>,
    pub expiry_ledger:     u32,
    pub status:            SwapStatus,
    pub amount_out:        i128,
    pub asset_in:          Address,
    pub asset_out:         Address,
}

#[contracttype]
pub struct SwapFairnessPublicInputs {
    pub intent_commitment: BytesN<32>,
    pub asset_in:          Address,
    pub asset_out:         Address,
    pub amount_out:        i128,
    pub min_amount_out:    i128,
}

#[contract]
pub struct ShieldedSwap;

#[contractimpl]
impl ShieldedSwap {

    pub fn initialize(env: Env, admin: Address) {
        env.storage().instance().set(&StorageKey::Admin, &admin);
    }

    pub fn commit_swap(
        env:               Env,
        nullifier_in:      BytesN<32>,
        intent_commitment: BytesN<32>,
        asset_in:          Address,
        asset_out:         Address,
        _commitment_proof: Bytes,
        expiry_ledger:     u32,
    ) -> BytesN<32> {
        assert!(expiry_ledger > env.ledger().sequence(), "expiry must be in the future");

        let swap_id: BytesN<32> = env.crypto().sha256(&intent_commitment.clone().into()).into();

        let state = SwapState {
            intent_commitment,
            nullifier_in,
            expiry_ledger,
            status: SwapStatus::Committed,
            amount_out: 0,
            asset_in,
            asset_out,
        };
        env.storage().instance().set(&StorageKey::SwapState(swap_id.clone()), &state);

        env.events().publish(
            (symbol_short!("zkella"), symbol_short!("commit")),
            (swap_id.clone(), expiry_ledger),
        );

        swap_id
    }

    pub fn execute_swap(
        env:        Env,
        swap_id:    BytesN<32>,
        amount_out: i128,
        relayer:    Address,
    ) {
        relayer.require_auth();
        assert!(
            env.storage().instance().has(&StorageKey::ApprovedRelayer(relayer.clone())),
            "relayer not approved"
        );

        let mut state: SwapState = env.storage().instance()
            .get(&StorageKey::SwapState(swap_id.clone())).expect("swap not found");
        assert!(state.status == SwapStatus::Committed, "swap not in committed state");
        assert!(env.ledger().sequence() <= state.expiry_ledger, "swap expired");

        state.status = SwapStatus::Executed;
        state.amount_out = amount_out;
        env.storage().instance().set(&StorageKey::SwapState(swap_id.clone()), &state);

        env.events().publish(
            (symbol_short!("zkella"), symbol_short!("exec")),
            (swap_id, amount_out),
        );
    }

    pub fn reveal_and_claim(
        env:            Env,
        swap_id:        BytesN<32>,
        out_commitment: BytesN<32>,
        encrypted_note: Bytes,
        _fairness_proof: Bytes,
        _fairness_pub:  SwapFairnessPublicInputs,
    ) -> u32 {
        let mut state: SwapState = env.storage().instance()
            .get(&StorageKey::SwapState(swap_id.clone())).expect("swap not found");
        assert!(state.status == SwapStatus::Executed, "swap not executed");

        // Verify fairness proof (M3)
        state.status = SwapStatus::Claimed;
        env.storage().instance().set(&StorageKey::SwapState(swap_id.clone()), &state);

        env.events().publish(
            (symbol_short!("zkella"), symbol_short!("claim")),
            (swap_id, out_commitment.clone(), encrypted_note),
        );

        // Return placeholder leaf index — CT-20 insertion handled in M3
        0u32
    }

    pub fn cancel_swap(env: Env, swap_id: BytesN<32>, _proof: Bytes) {
        let mut state: SwapState = env.storage().instance()
            .get(&StorageKey::SwapState(swap_id.clone())).expect("swap not found");
        assert!(
            state.status == SwapStatus::Committed
            && env.ledger().sequence() > state.expiry_ledger,
            "cannot cancel"
        );
        state.status = SwapStatus::Cancelled;
        env.storage().instance().set(&StorageKey::SwapState(swap_id.clone()), &state);

        env.events().publish(
            (symbol_short!("zkella"), symbol_short!("cancel")),
            swap_id,
        );
    }

    pub fn set_relayer(env: Env, relayer: Address, approved: bool) {
        let admin: Address = env.storage().instance().get(&StorageKey::Admin).unwrap();
        admin.require_auth();
        if approved {
            env.storage().instance().set(&StorageKey::ApprovedRelayer(relayer), &true);
        } else {
            env.storage().instance().remove(&StorageKey::ApprovedRelayer(relayer));
        }
    }
}
