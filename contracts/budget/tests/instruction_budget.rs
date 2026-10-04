use soroban_sdk::{testutils::{Address as _, Ledger as _}, Address, Bytes, BytesN, Env};
use zkella_compliance::{ComplianceContractClient};
use zkella_governance::ZKELLAGovernanceClient;
use zkella_swap::ShieldedSwapClient;
use zkella_token::ShieldedTokenClient;
use zkella_verifier_interface::CircuitType;
use zkella_viewing_keys::ViewingKeyRegistryClient;

const MAINNET_BUDGET: u64 = 400_000_000;

const TOKEN_WASM: &[u8] = include_bytes!("../../target/wasm32v1-none/release/zkella_token.wasm");
const VERIFIER_WASM: &[u8] = include_bytes!("../../target/wasm32v1-none/release/zkella_verifier.wasm");
const SWAP_WASM: &[u8] = include_bytes!("../../target/wasm32v1-none/release/zkella_swap.wasm");
const GOVERNANCE_WASM: &[u8] = include_bytes!("../../target/wasm32v1-none/release/zkella_governance.wasm");
const COMPLIANCE_WASM: &[u8] = include_bytes!("../../target/wasm32v1-none/release/zkella_compliance.wasm");
const VIEWING_KEYS_WASM: &[u8] = include_bytes!("../../target/wasm32v1-none/release/zkella_viewing_keys.wasm");

fn new_env() -> Env {
    let env = Env::default();
    env.cost_estimate().budget().reset_limits(2_000_000_000, 100_000_000);
    env.mock_all_auths();
    env
}

fn measure<T>(env: &Env, call: impl FnOnce() -> T) -> (T, u64) {
    env.cost_estimate().budget().reset_tracker();
    let out = call();
    (out, env.cost_estimate().budget().cpu_instruction_cost())
}

fn record(rows: &mut std::vec::Vec<(&'static str, &'static str, u64)>, contract: &'static str, name: &'static str, cost: u64) {
    assert!(cost < MAINNET_BUDGET, "{contract}::{name} used {cost} instructions, over the {MAINNET_BUDGET} mainnet budget");
    rows.push((contract, name, cost));
}

fn verifier_with(env: &Env, admin: &Address) -> Address {
    let verifier = env.register(VERIFIER_WASM, ());
    zkella_verifier::VerifierContractClient::new(env, &verifier).initialize(admin);
    verifier
}

#[test]
fn instruction_budget_for_every_non_proof_entrypoint() {
    let mut rows: std::vec::Vec<(&'static str, &'static str, u64)> = std::vec::Vec::new();

    {
        let env = new_env();
        let admin = Address::generate(&env);
        let verifier = verifier_with(&env, &admin);
        let token = env.register(TOKEN_WASM, ());
        let t = ShieldedTokenClient::new(&env, &token);
        t.initialize(&admin, &verifier);
        let asset = Address::generate(&env);
        let relayer = Address::generate(&env);
        let new_admin = Address::generate(&env);

        let (_, c) = measure(&env, || t.set_min_shield_amount(&1_000));
        record(&mut rows, "token", "set_min_shield_amount", c);
        let (_, c) = measure(&env, || t.min_shield_amount());
        record(&mut rows, "token", "min_shield_amount", c);
        let (_, c) = measure(&env, || t.set_asset_approved(&asset, &true));
        record(&mut rows, "token", "set_asset_approved", c);
        let (_, c) = measure(&env, || t.is_asset_approved(&asset));
        record(&mut rows, "token", "is_asset_approved", c);
        let (_, c) = measure(&env, || t.set_relayer(&relayer, &true));
        record(&mut rows, "token", "set_relayer", c);
        let (_, c) = measure(&env, || t.is_approved_relayer(&relayer));
        record(&mut rows, "token", "is_approved_relayer", c);
        let (_, c) = measure(&env, || t.merkle_root());
        record(&mut rows, "token", "merkle_root", c);
        let (_, c) = measure(&env, || t.leaf_count());
        record(&mut rows, "token", "leaf_count", c);
        let (_, c) = measure(&env, || t.shielded_supply(&asset));
        record(&mut rows, "token", "shielded_supply", c);
        let (_, c) = measure(&env, || t.is_spent(&BytesN::from_array(&env, &[0u8; 32])));
        record(&mut rows, "token", "is_spent", c);
        let (_, c) = measure(&env, || t.pause());
        record(&mut rows, "token", "pause", c);
        let (_, c) = measure(&env, || t.unpause());
        record(&mut rows, "token", "unpause", c);
        let (_, c) = measure(&env, || t.transfer_admin(&new_admin));
        record(&mut rows, "token", "transfer_admin", c);
        let (_, c) = measure(&env, || t.accept_admin());
        record(&mut rows, "token", "accept_admin", c);
    }

    {
        let env = new_env();
        let admin = Address::generate(&env);
        let verifier = env.register(VERIFIER_WASM, ());
        let v = zkella_verifier::VerifierContractClient::new(&env, &verifier);
        let (_, c) = measure(&env, || v.initialize(&admin));
        record(&mut rows, "verifier", "initialize", c);
        let vk = Bytes::from_array(&env, &[0u8; 64 + 384 + 64 * 2]);
        let (_, c) = measure(&env, || v.register_verifying_key(&zkella_verifier::CircuitType::Shield, &vk));
        record(&mut rows, "verifier", "register_verifying_key", c);
        let vk2 = Bytes::from_array(&env, &[1u8; 64 + 384 + 64 * 2]);
        let (_, c) = measure(&env, || v.update_verifying_key(&zkella_verifier::CircuitType::Shield, &vk2));
        record(&mut rows, "verifier", "update_verifying_key", c);
        let (_, c) = measure(&env, || v.get_verifying_key(&zkella_verifier::CircuitType::Shield));
        record(&mut rows, "verifier", "get_verifying_key", c);
        let (_, c) = measure(&env, || v.revoke_previous_vk(&zkella_verifier::CircuitType::Shield));
        record(&mut rows, "verifier", "revoke_previous_vk", c);
    }

    {
        let env = new_env();
        let admin = Address::generate(&env);
        let gov = env.register(GOVERNANCE_WASM, ());
        let verifier = verifier_with(&env, &gov);
        let g = ZKELLAGovernanceClient::new(&env, &gov);
        let guardian = Address::generate(&env);
        let token = env.register(TOKEN_WASM, ());
        ShieldedTokenClient::new(&env, &token).initialize(&gov, &verifier);
        let (_, c) = measure(&env, || g.initialize(&admin, &verifier, &guardian, &token));
        record(&mut rows, "governance", "initialize", c);
        let (_, c) = measure(&env, || g.timelock_ledgers());
        record(&mut rows, "governance", "timelock_ledgers", c);
        let vk = Bytes::from_array(&env, &[0u8; 64 + 384 + 64 * 2]);
        let (_, c) = measure(&env, || g.queue_vk_update(&CircuitType::Shield, &vk));
        record(&mut rows, "governance", "queue_vk_update", c);
        let (_, c) = measure(&env, || g.cancel_vk_update(&CircuitType::Shield));
        record(&mut rows, "governance", "cancel_vk_update", c);
        g.queue_vk_update(&CircuitType::Shield, &vk);
        let delay = g.timelock_ledgers();
        env.ledger().with_mut(|l| l.sequence_number += delay);
        let (_, c) = measure(&env, || g.execute_vk_update(&CircuitType::Shield));
        record(&mut rows, "governance", "execute_vk_update", c);
        let (_, c) = measure(&env, || g.revoke_previous_vk(&CircuitType::Shield));
        record(&mut rows, "governance", "revoke_previous_vk", c);
        let new_admin = Address::generate(&env);
        let (_, c) = measure(&env, || g.transfer_admin(&new_admin));
        record(&mut rows, "governance", "transfer_admin", c);
        let (_, c) = measure(&env, || g.accept_admin());
        record(&mut rows, "governance", "accept_admin", c);
        let (_, c) = measure(&env, || g.queue_min_shield_amount(&2_000));
        record(&mut rows, "governance", "queue_min_shield_amount", c);
        let delay = g.timelock_ledgers();
        env.ledger().with_mut(|li| { li.sequence_number += delay; });
        let (_, c) = measure(&env, || g.execute_min_shield_amount());
        record(&mut rows, "governance", "execute_min_shield_amount", c);
        let (_, c) = measure(&env, || g.guardian_cancel_min_shield());
        record(&mut rows, "governance", "guardian_cancel_min_shield", c);
    }

    {
        let env = new_env();
        let admin = Address::generate(&env);
        let verifier = verifier_with(&env, &admin);
        let compliance = env.register(COMPLIANCE_WASM, ());
        let c_client = ComplianceContractClient::new(&env, &compliance);
        let (_, c) = measure(&env, || c_client.initialize(&admin, &verifier));
        record(&mut rows, "compliance", "initialize", c);
        let owner = Address::generate(&env);
        let (_, c) = measure(&env, || c_client.get_compliance_proof(&owner));
        record(&mut rows, "compliance", "get_compliance_proof", c);
    }

    {
        let env = new_env();
        let reg = env.register(VIEWING_KEYS_WASM, ());
        let vk = ViewingKeyRegistryClient::new(&env, &reg);
        let owner = Address::generate(&env);
        let commitment = BytesN::from_array(&env, &[7u8; 32]);
        let (_, c) = measure(&env, || vk.register(&owner, &commitment, &100));
        record(&mut rows, "viewing_keys", "register", c);
        let (_, c) = measure(&env, || vk.get_viewing_key_commitment(&owner));
        record(&mut rows, "viewing_keys", "get_viewing_key_commitment", c);
    }

    {
        let env = new_env();
        let admin = Address::generate(&env);
        let verifier = verifier_with(&env, &admin);
        let token = env.register(TOKEN_WASM, ());
        ShieldedTokenClient::new(&env, &token).initialize(&admin, &verifier);
        let swap = env.register(SWAP_WASM, ());
        let s = ShieldedSwapClient::new(&env, &swap);
        let (_, c) = measure(&env, || s.initialize(&admin, &verifier, &token));
        record(&mut rows, "swap", "initialize", c);
        let relayer = Address::generate(&env);
        let (_, c) = measure(&env, || s.set_relayer(&relayer, &true));
        record(&mut rows, "swap", "set_relayer", c);
    }

    std::println!("INSTRUCTION_BUDGET contract,entrypoint,cpu_instructions,percent_of_mainnet_budget");
    for (contract, name, cost) in &rows {
        std::println!(
            "INSTRUCTION_BUDGET {contract},{name},{cost},{:.2}",
            *cost as f64 * 100.0 / MAINNET_BUDGET as f64
        );
    }
}
