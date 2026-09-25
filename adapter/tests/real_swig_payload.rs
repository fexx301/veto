use {
    solana_sdk::{instruction::Instruction, pubkey::Pubkey},
    swig_interface::{program_id, SignV2Instruction},
    veto_swig_gate::{
        inner_account_keys, reject_later_swig_instruction_list, validate_next_swig_instruction,
        validate_swig_account_binding,
    },
};

fn gate_proof(config: Pubkey, wallet: Pubkey) -> Instruction {
    Instruction {
        program_id: Pubkey::new_unique(),
        accounts: vec![
            solana_sdk::instruction::AccountMeta::new_readonly(config, false),
            solana_sdk::instruction::AccountMeta::new_readonly(wallet, false),
        ],
        data: vec![2],
    }
}

fn sign_v2_for(target: Pubkey) -> Instruction {
    let config = Pubkey::new_unique();
    let wallet = Pubkey::new_unique();
    let payer = Pubkey::new_unique();
    let target_call = Instruction {
        program_id: target,
        accounts: vec![],
        data: vec![42],
    };
    SignV2Instruction::new_program_exec(
        config,
        wallet,
        payer,
        gate_proof(config, wallet),
        target_call,
        1,
    )
    .unwrap()
    .pop()
    .unwrap()
}

fn sign_v2_for_with_explicit_proof_index(target: Pubkey, proof_index: u8) -> Instruction {
    let config = Pubkey::new_unique();
    let wallet = Pubkey::new_unique();
    let payer = Pubkey::new_unique();
    let target_call = Instruction {
        program_id: target,
        accounts: vec![],
        data: vec![42],
    };
    SignV2Instruction::new_program_exec_with_ix_index(
        config,
        wallet,
        payer,
        gate_proof(config, wallet),
        target_call,
        1,
        proof_index,
    )
    .unwrap()
    .pop()
    .unwrap()
}

#[test]
fn accepts_the_real_swig_builder_payload_for_the_pinned_target() {
    let target = Pubkey::new_unique();
    let sign = sign_v2_for(target);
    validate_next_swig_instruction(&sign, &program_id(), &target).unwrap();
}

#[test]
fn rejects_reusing_the_proof_with_another_inner_target() {
    let approved_target = Pubkey::new_unique();
    let substituted_target = Pubkey::new_unique();
    let sign = sign_v2_for(substituted_target);
    let error = validate_next_swig_instruction(&sign, &program_id(), &approved_target).unwrap_err();
    assert_eq!(error, solana_sdk::program_error::ProgramError::Custom(7));
}

#[test]
fn rejects_a_non_swig_instruction_even_if_it_mentions_the_target() {
    let target = Pubkey::new_unique();
    let not_swig = Instruction {
        program_id: Pubkey::new_unique(),
        accounts: vec![solana_sdk::instruction::AccountMeta::new_readonly(
            target, false,
        )],
        data: vec![11, 0, 0, 0, 0, 0, 0, 0, 1],
    };
    let error = validate_next_swig_instruction(&not_swig, &program_id(), &target).unwrap_err();
    assert_eq!(error, solana_sdk::program_error::ProgramError::Custom(5));
}

#[test]
fn rejects_explicit_earlier_proof_indices_from_the_real_swig_builder() {
    let target = Pubkey::new_unique();
    let sign = sign_v2_for_with_explicit_proof_index(target, 0);
    let error = validate_next_swig_instruction(&sign, &program_id(), &target).unwrap_err();
    assert_eq!(error, solana_sdk::program_error::ProgramError::Custom(8));
}

#[test]
fn rejects_a_second_real_swig_call_that_could_reuse_the_first_proof() {
    let target = Pubkey::new_unique();
    let later_sign = sign_v2_for_with_explicit_proof_index(target, 0);
    let error = reject_later_swig_instruction_list(&[later_sign], &program_id()).unwrap_err();
    assert_eq!(error, solana_sdk::program_error::ProgramError::Custom(10));
}

#[test]
fn binds_the_gate_proof_to_the_config_and_wallet_of_the_real_swig_call() {
    let target = Pubkey::new_unique();
    let config = Pubkey::new_unique();
    let wallet = Pubkey::new_unique();
    let payer = Pubkey::new_unique();
    let target_call = Instruction {
        program_id: target,
        accounts: vec![],
        data: vec![42],
    };
    let sign = SignV2Instruction::new_program_exec(
        config,
        wallet,
        payer,
        gate_proof(config, wallet),
        target_call,
        1,
    )
    .unwrap()
    .pop()
    .unwrap();

    validate_swig_account_binding(&sign, &config, &wallet).unwrap();

    let mut wrong_config = sign.clone();
    wrong_config.accounts[0].pubkey = Pubkey::new_unique();
    let error = validate_swig_account_binding(&wrong_config, &config, &wallet).unwrap_err();
    assert_eq!(error, solana_sdk::program_error::ProgramError::Custom(12));

    let mut wrong_wallet = sign;
    wrong_wallet.accounts[1].pubkey = Pubkey::new_unique();
    let error = validate_swig_account_binding(&wrong_wallet, &config, &wallet).unwrap_err();
    assert_eq!(error, solana_sdk::program_error::ProgramError::Custom(13));
}

#[test]
fn exposes_every_account_the_delegated_call_receives() {
    let config = Pubkey::new_unique();
    let wallet = Pubkey::new_unique();
    let router = Pubkey::new_unique();
    let downstream = Pubkey::new_unique();
    let source = Pubkey::new_unique();
    let merchant = Pubkey::new_unique();
    let target_call = Instruction {
        program_id: router,
        accounts: vec![
            solana_sdk::instruction::AccountMeta::new(source, false),
            solana_sdk::instruction::AccountMeta::new(merchant, false),
            solana_sdk::instruction::AccountMeta::new_readonly(wallet, true),
            solana_sdk::instruction::AccountMeta::new_readonly(downstream, false),
        ],
        data: vec![7],
    };
    let sign = SignV2Instruction::new_program_exec(config, wallet, Pubkey::new_unique(), gate_proof(config, wallet), target_call, 1)
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(inner_account_keys(&sign).unwrap(), vec![source, merchant, wallet, downstream]);
}
