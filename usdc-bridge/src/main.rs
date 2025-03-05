use {
    alloy::{
        consensus::constants::ETH_TO_WEI,
        hex,
        network::{EthereumWallet, TransactionBuilder},
        primitives::{address, Address, U128, U256, U64},
        providers::{Provider, ProviderBuilder},
        rpc::types::TransactionRequest,
        signers::local::LocalSigner,
        sol,
        transports::http::reqwest,
    },
    serde_json::json,
    solana_client::nonblocking::rpc_client::RpcClient,
    solana_sdk::{
        commitment_config::CommitmentConfig, native_token::LAMPORTS_PER_SOL, pubkey::Pubkey,
        signature::Keypair, signer::Signer, transaction::VersionedTransaction,
    },
    spl_associated_token_account::get_associated_token_address,
    std::str::FromStr,
};

#[tokio::main]
async fn main() {
    let sol_rpc = "https://api.mainnet-beta.solana.com";
    let client_sol = RpcClient::new_with_commitment(
        sol_rpc.to_string(),
        CommitmentConfig::confirmed(), // TODO what commitment level should we use?
    );

    let account_sol =
        Keypair::from_base58_string(&std::fs::read_to_string("sol-account1.key").unwrap());
    let account_eth =
        LocalSigner::from_str(&std::fs::read_to_string("eth-account1.key").unwrap()).unwrap();

    let eth_rpc = "https://rpc.walletconnect.org/v1?projectId=eb9a267f4dc99a03dc2d570916f55ec2&chainId=eip155:8453";
    let client_eth = ProviderBuilder::new()
        .wallet(EthereumWallet::new(account_eth.clone()))
        .on_http(eth_rpc.parse().unwrap());

    println!("sol_account public key: {}", account_sol.pubkey());
    println!("eth_account address: {}", account_eth.address());

    // Check SOL balances
    let account_sol_balance = client_sol.get_balance(&account_sol.pubkey()).await.unwrap();
    let account_eth_balance = client_eth.get_balance(account_eth.address()).await.unwrap();

    println!(
        "SOL account SOL balance: {} SOL",
        account_sol_balance as f64 / LAMPORTS_PER_SOL as f64
    );
    println!(
        "ETH account ETH balance: {} ETH",
        account_eth_balance.to::<u128>() as f64 / ETH_TO_WEI as f64
    );

    // USDC token mint on devnet
    let usdc_mint = Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap();

    // Amount to transfer (1 USDC = 1_000_000 lamports)
    let amount = 1_000_000; // 1 USDC

    // Get associated token accounts
    let account_sol_token_account = get_associated_token_address(&account_sol.pubkey(), &usdc_mint);

    let account_sol_usdc_balance = client_sol
        .get_token_account_balance(&account_sol_token_account)
        .await
        .unwrap();

    sol! {
        #[sol(rpc)]
        contract ERC20 {
            function name() public view returns (string);
            function symbol() public view returns (string);
            function decimals() public view returns (uint8);
            function totalSupply() public view returns (uint256);
            function balanceOf(address _owner) public view returns (uint256 balance);
            function transfer(address _to, uint256 _value) public returns (bool success);
            function transferFrom(address _from, address _to, uint256 _value) public returns (bool success);
            function approve(address _spender, uint256 _value) public returns (bool success);
            function allowance(address _owner, address _spender) public view returns (uint256 remaining);
        }
    }

    const USDC_CONTRACT_BASE: Address = address!("833589fCD6eDb6E08f4c7C32D4f71b54bdA02913");
    let usdc_contract = ERC20::new(USDC_CONTRACT_BASE, client_eth.clone());

    let account_eth_usdc_balance = usdc_contract
        .balanceOf(account_eth.address())
        .call()
        .await
        .unwrap()
        .balance;
    let account_eth_usdc_balance_float = account_eth_usdc_balance.to::<u128>() as f64 / 1000000.0;

    println!(
        "Account1 USDC balance: {}",
        account_sol_usdc_balance.ui_amount.unwrap_or(0.0)
    );
    println!("Account2 USDC balance: {}", account_eth_usdc_balance_float);

    let sol_to_eth =
        account_sol_usdc_balance.ui_amount.unwrap_or(0.0) > account_eth_usdc_balance_float;

    if sol_to_eth {
        // Check if sender has enough USDC
        if account_sol_usdc_balance.amount.parse::<u64>().unwrap() < amount {
            println!("Error: Sender doesn't have enough USDC");
            return;
        }

        let quote = reqwest::Client::new()
            .get("https://li.quest/v1/quote")
            .query(&json!({
                "fromChain": "SOL",
                "toChain": "BAS",
                "fromToken": usdc_mint.to_string(),
                "toToken": USDC_CONTRACT_BASE.to_string(),
                "fromAmount": amount,
                "fromAddress": account_sol.pubkey().to_string(),
                "toAddress": account_eth.address().to_string(),
            }))
            .send()
            .await
            .unwrap()
            .json::<serde_json::Value>()
            .await
            .unwrap();
        println!("Quote: {}", serde_json::to_string_pretty(&quote).unwrap());

        let route = quote["action"].clone();
        assert_eq!(
            route["fromAddress"].as_str().unwrap(),
            account_sol.pubkey().to_string()
        );
        assert_eq!(route["fromChainId"].as_u64().unwrap(), 1151111081099710);
        assert_eq!(route["fromAmount"].as_str().unwrap(), "1000000");
        let from_token = route["fromToken"].clone();
        assert_eq!(
            from_token["address"].as_str().unwrap(),
            usdc_mint.to_string()
        );
        assert_eq!(from_token["chainId"].as_u64().unwrap(), 1151111081099710);
        assert_eq!(from_token["symbol"].as_str().unwrap(), "USDC");
        assert_eq!(from_token["decimals"].as_u64().unwrap(), 6);

        let to_token = route["toToken"].clone();
        assert_eq!(
            to_token["address"].as_str().unwrap(),
            USDC_CONTRACT_BASE.to_string()
        );
        assert_eq!(to_token["chainId"].as_u64().unwrap(), 8453);
        assert_eq!(to_token["symbol"].as_str().unwrap(), "USDC");
        assert_eq!(to_token["decimals"].as_u64().unwrap(), 6);

        println!("Preparing transfer transaction...");
        let data = data_encoding::BASE64
            .decode(
                quote
                    .get("transactionRequest")
                    .unwrap()
                    .get("data")
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .as_bytes(),
            )
            .unwrap();

        // Get recent blockhash
        // let recent_blockhash = client_sol.get_latest_blockhash().await.unwrap();

        let tx = bincode::deserialize::<VersionedTransaction>(&data).unwrap();
        println!("tx: {:?}", tx);

        let serialized_message = tx.message.serialize();
        let signature = account_sol.sign_message(&serialized_message);
        println!("signature: {:?}", signature);
        let transaction = VersionedTransaction {
            signatures: vec![signature],
            message: tx.message,
        };

        println!("Sending transaction...");
        // Send and confirm transaction
        match client_sol.send_and_confirm_transaction(&transaction).await {
            Ok(signature) => println!("Transfer successful! Signature: {}", signature),
            Err(e) => println!("Error sending transaction: {}", e),
        }

        // TODO await bridge status
    } else {
        // Check if sender has enough USDC
        if account_eth_balance < U256::from(amount) {
            println!("Error: Sender doesn't have enough USDC");
            return;
        }

        let quote = reqwest::Client::new()
            .get("https://li.quest/v1/quote")
            .query(&json!({
                "fromChain": "BAS",
                "toChain": "SOL",
                "fromToken": USDC_CONTRACT_BASE.to_string(),
                "toToken": usdc_mint.to_string(),
                "fromAmount": amount,
                "fromAddress": account_eth.address().to_string(),
                "toAddress": account_sol.pubkey().to_string(),
            }))
            .send()
            .await
            .unwrap()
            .json::<serde_json::Value>()
            .await
            .unwrap();
        println!("Quote: {}", serde_json::to_string_pretty(&quote).unwrap());

        let route = quote["action"].clone();
        assert_eq!(
            route["fromAddress"].as_str().unwrap(),
            account_eth.address().to_string()
        );
        assert_eq!(route["fromChainId"].as_u64().unwrap(), 8453);
        assert_eq!(route["fromAmount"].as_str().unwrap(), "1000000");
        let from_token = route["fromToken"].clone();
        assert_eq!(
            from_token["address"].as_str().unwrap(),
            USDC_CONTRACT_BASE.to_string()
        );
        assert_eq!(from_token["chainId"].as_u64().unwrap(), 8453);
        assert_eq!(from_token["symbol"].as_str().unwrap(), "USDC");
        assert_eq!(from_token["decimals"].as_u64().unwrap(), 6);

        let to_token = route["toToken"].clone();
        assert_eq!(to_token["address"].as_str().unwrap(), usdc_mint.to_string());
        assert_eq!(to_token["chainId"].as_u64().unwrap(), 1151111081099710);
        assert_eq!(to_token["symbol"].as_str().unwrap(), "USDC");
        assert_eq!(to_token["decimals"].as_u64().unwrap(), 6);

        let transaction_request = quote["transactionRequest"].clone();

        let bridge_contract = transaction_request["to"].as_str().unwrap().parse().unwrap();

        let allowance = usdc_contract
            .allowance(account_eth.address(), bridge_contract)
            .call()
            .await
            .unwrap()
            .remaining;
        println!("Allowance: {}", allowance);
        if allowance < U256::from(amount) {
            assert!(usdc_contract
                .approve(bridge_contract, U256::from(amount * 2))
                .send()
                .await
                .unwrap()
                .get_receipt()
                .await
                .unwrap()
                .status());
        }

        let transaction_request = TransactionRequest::default()
            .with_chain_id(transaction_request["chainId"].as_u64().unwrap())
            .with_from(
                transaction_request["from"]
                    .as_str()
                    .unwrap()
                    .parse()
                    .unwrap(),
            )
            .with_to(bridge_contract)
            .with_value(
                transaction_request["value"]
                    .as_str()
                    .unwrap()
                    .parse()
                    .unwrap(),
            )
            .with_input(hex::decode(transaction_request["data"].as_str().unwrap()).unwrap())
            .with_gas_price(
                transaction_request["gasPrice"]
                    .as_str()
                    .unwrap()
                    .parse::<U128>()
                    .unwrap()
                    .to(),
            )
            .with_gas_limit(
                transaction_request["gasLimit"]
                    .as_str()
                    .unwrap()
                    .parse::<U64>()
                    .unwrap()
                    .to(),
            );
        let receipt = client_eth
            .send_transaction(transaction_request)
            .await
            .unwrap()
            .get_receipt()
            .await
            .unwrap();
        println!(
            "Receipt: {}",
            serde_json::to_string_pretty(&receipt).unwrap()
        );
        assert!(receipt.status());

        // TODO await bridge status
    }
}
