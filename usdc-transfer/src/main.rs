use {
    solana_client::nonblocking::rpc_client::RpcClient,
    solana_sdk::{
        commitment_config::CommitmentConfig, native_token::LAMPORTS_PER_SOL, pubkey::Pubkey,
        signature::Keypair, signer::Signer, transaction::Transaction,
    },
    spl_associated_token_account::get_associated_token_address,
    spl_token::instruction as token_instruction,
    std::str::FromStr,
};

#[tokio::main]
async fn main() {
    let client = RpcClient::new_with_commitment(
        "https://api.mainnet-beta.solana.com".to_string(),
        CommitmentConfig::confirmed(), // TODO what commitment level should we use?
    );

    let account1 = Keypair::from_base58_string(&std::fs::read_to_string("sol-account2.key").unwrap());
    let account2 = Keypair::from_base58_string(&std::fs::read_to_string("sol-account1.key").unwrap());

    println!("Account1 public key: {}", account1.pubkey());
    println!("Account2 public key: {}", account2.pubkey());

    // Check SOL balances
    let account1_balance = client.get_balance(&account1.pubkey()).await.unwrap();
    let account2_balance = client.get_balance(&account2.pubkey()).await.unwrap();

    println!(
        "Account1 SOL balance: {} SOL",
        account1_balance as f64 / LAMPORTS_PER_SOL as f64
    );
    println!(
        "Account2 SOL balance: {} SOL",
        account2_balance as f64 / LAMPORTS_PER_SOL as f64
    );

    // USDC token mint on devnet
    let usdc_mint = Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap();

    // Amount to transfer (1 USDC = 1_000_000 lamports)
    let amount = 1_000_000; // 1 USDC

    // Get associated token accounts
    let account1_token_account = get_associated_token_address(&account1.pubkey(), &usdc_mint);
    let account2_token_account = get_associated_token_address(&account2.pubkey(), &usdc_mint);

    let account1_usdc_balance = client
        .get_token_account_balance(&account1_token_account)
        .await
        .unwrap();
    let account2_usdc_balance = client
        .get_token_account_balance(&account2_token_account)
        .await
        .unwrap();

    println!(
        "Account1 USDC balance: {}",
        account1_usdc_balance.ui_amount.unwrap_or(0.0)
    );
    println!(
        "Account2 USDC balance: {}",
        account2_usdc_balance.ui_amount.unwrap_or(0.0)
    );

    let (
        (sender, sender_token_account, sender_usdc_balance),
        (receiver, receiver_token_account, receiver_usdc_balance),
    ) = if account1_usdc_balance.ui_amount.unwrap_or(0.0)
        > account2_usdc_balance.ui_amount.unwrap_or(0.0)
    {
        (
            (account1, account1_token_account, account1_usdc_balance),
            (account2, account2_token_account, account2_usdc_balance),
        )
    } else {
        (
            (account2, account2_token_account, account2_usdc_balance),
            (account1, account1_token_account, account1_usdc_balance),
        )
    };

    println!("Sender USDC balance: {}", sender_usdc_balance.amount);
    println!("Receiver USDC balance: {}", receiver_usdc_balance.amount);

    // Check if sender has enough USDC
    if sender_usdc_balance.amount.parse::<u64>().unwrap() < amount {
        println!("Error: Sender doesn't have enough USDC");
        return;
    }

    println!("Checking receiver's token account...");
    // Create receiver's associated token account if it doesn't exist
    if client.get_account(&receiver.pubkey()).await.is_err() {
        println!("Creating receiver's token account...");
        let create_ata_ix =
            spl_associated_token_account::instruction::create_associated_token_account(
                &sender.pubkey(),
                &receiver.pubkey(),
                &usdc_mint,
                &spl_token::id(),
            );

        let recent_blockhash = client.get_latest_blockhash().await.unwrap();
        let create_ata_tx = Transaction::new_signed_with_payer(
            &[create_ata_ix],
            Some(&sender.pubkey()),
            &[&sender],
            recent_blockhash,
        );

        client
            .send_and_confirm_transaction(&create_ata_tx)
            .await
            .unwrap();
    }

    println!("Preparing transfer transaction...");
    // Create transfer instruction
    let transfer_ix = token_instruction::transfer(
        &spl_token::id(),
        &sender_token_account,
        &receiver_token_account,
        &sender.pubkey(),
        &[],
        amount,
    )
    .unwrap();

    // Get recent blockhash
    let recent_blockhash = client.get_latest_blockhash().await.unwrap();

    // Create and sign transaction
    let transaction = Transaction::new_signed_with_payer(
        &[transfer_ix],
        Some(&sender.pubkey()),
        &[&sender],
        recent_blockhash,
    );

    println!("Sending transaction...");
    // Send and confirm transaction
    match client.send_and_confirm_transaction(&transaction).await {
        Ok(signature) => println!("Transfer successful! Signature: {}", signature),
        Err(e) => println!("Error sending transaction: {}", e),
    }
}
