# Genesis File

---

## **What is a Genesis File**

A genesis file is a JSON file which defines the initial state of your blockchain. It can be seen as height 0 of your blockchain. The first block, at height 1, will reference the genesis file as its parent.

The state defined in the genesis file contains all the necessary information, like initial token allocation, genesis time, default parameters, and more. Let us break down these information.

## **Genesis Time and Chain_id**

The genesis_time is defined at the top of the genesis file. It is a UTC timestamps which specifies when the blockchain is due to start. At this time, genesis validators are supposed to come online and start participating in the consensus process. The blockchain starts when more than 2/3rd of the genesis validators (weighted by voting power) are online.

The chain_id is a unique identifier for your chain. It helps differentiate between different chains using the same version of the software.

## **Consensus Parameters**

Next, the genesis file defines consensus parameters. Consensus parameters regroup all the parameters that are related to the consensus layer, which is Tendermint in the case of Chronic Chain. Let us look at these parameters:

- block
  - max_bytes: Maximum number of bytes per block.
  - max_gas: Gas limit per block. Each transaction included in the block will consume some gas. The total gas used by transactions included in a block cannot exceed this limit.
- evidence
  - max_age: An evidence is a proof that a validator signed two different blocks at the same height (and round). This is an explicitly malicious behaviour that is punished at the state-machine level. The max_age defines the maximum number of
- validator
  - pub_key_types: The types of pubkey (ed25519, secp256k1, ...) that are accepted for validators. Currently only ed25519 is accepted.

## **Application State**

The application state defines the initial state of the state-machine.

### **Genesis Accounts**

In this section, initial allocation of tokens is defined. It is possible to add accounts manually by directly editing the genesis file, but it is also possible to use the following command:

```sh
terpd add-genesis-account <account-address> <amount><denom>
```

This command creates an item in the accounts list, under the app_state section.

- `sequence_number`: This number is used to count the number of transactions sent by this account. It is incremented each time a transaction is included in a block, and used to prevent replay attacks. Initial value is 0.
- `account_number`: Unique identifier for the account. It is generated the first time a transaction including this account is included in a block.
- `original_vesting`: Vesting is natively supported by terpd. You can define an amount of token owned by the account that needs to be vested for a period of time before they can be transferred. Vested tokens can be delegated. Default value is null.
- `delegated_free`: Amount of delegated tokens that can be transferred after they've been vested. Most of the time, will be null in genesis.
- `delegated_vesting`: Amount of delegated tokens that are still vesting. Most of the time, will be null in genesis.
- `start_time`: Block at which the vesting period starts. 0 most of the time in genesis.
- `end_time`: Block at which the vesting period ends. 0 if no vesting for this account.

### **Bank**

The bank module handles tokens. The only parameter that needs to be defined in this section is whether transfers are enabled at genesis or not.

### **Staking**

The staking module handles the bulk of the Proof-of-Stake logic of the state-machine. This section should look like the following:

Let us break down the parameters:

 -`pool`
     - `not_bonded_tokens:` Defines the amount of tokens not bonded (i.e. delegated) in genesis. Generally, it equals the total supply of the staking token (uatom in this example).
     - `bonded_tokens:` Amount of bonded tokens in genesis. Generally 0.

- `params`
  - `unbonding_time:` Time in
  - `max_validators:` Maximum number of active validators.
  - `max_entries:` Maximum unbonding delegations and redelegations between a particular pair of delegator / validator.
  - `bond_denom:` Denomination of the staking token.
- `last_total_power:` Total amount of voting power. Generally 0 in genesis (except if genesis was generated using a previous state).
- `last_validator_powers:` Power of each validator in last known state. Generally null in genesis (except if genesis was generated using a previous state).
- `validators:` List of last known validators. Generally null in genesis (except if genesis was generated using a previous state).
- `bonds:` List of last known delegation. Generally null in genesis (except if genesis was generated using a previous state).
- `unbonding_delegations:` List of last known unbonding delegations. Generally null in genesis (except if genesis was generated using a previous state).
- `redelegations:` List of last known redelegations. Generally null in genesis (except if genesis was generated using a previous state).
- `exported:` Wether this genesis was generated using the export of a previous state.

### **Mint**

The mint module governs the logic of inflating the supply of token. The mint section in the genesis file looks like the follwing:

Let us break down the parameters:

- `minter`
  - `inflation:` Initial yearly percentage of increase in the total supply of staking token, compounded weekly. A 0.070000000000000000 value means the target is 7% yearly inflation, compounded weekly.
  - `annual_provisions:` Calculated each block. Initialize at 0.000000000000000000.
- params
  - `mint_denom:` Denom of the staking token that is inflated.
  - `inflation_rate_change:` Max yearly change in inflation.
  - `inflation_max:` Maximum level of inflation.
  - `inflation_min:` Minimum level of inflation.
  - `goal_bonded:` Percentage of the total supply that is targeted to be bonded. If the percentage of bonded staking tokens is below this target, the inflation increases (following inflation_rate_change) until it reaches inflation_max. If the percentage of bonded staking tokens is above this target, the inflation decreases (following inflation_rate_change) until it reaches inflation_min.
  - `blocks_per_year:` Estimation of the amount of blocks per year. Used to compute the block reward coming from inflated staking token (called block provisions).

### **Distribution**

The distribution module handles the logic of distribution block provisions and fees to validators and delegators. The distribution section in the genesis file looks like the follwing:

Let us break down the parameters:

- `fee_pool`
  - `community_pool:` The community pool is a pool of tokens that can be used to pay for bounties. It is allocated via governance proposals. Generally null in genesis.
- `community_tax:` The tax percentage on fees and block rewards that goes to the community pool.
- `base_proposer_reward:` Base bonus on transaction fees collected in a valid block that goes to the proposer of block. If value is 0.010000000000000000, 1% of the fees go to the proposer.
- `bonus_proposer_reward:` Max bonus on transaction fees collected in a valid block that goes to the proposer of block. The bonus depends on the number of precommits the proposer includes. If the proposer includes 2/3rd precommits weighted by voting power (minimum for the block to be valid), they get a bonus of base_proposer_reward. This bonus increases linearly up to bonus_proposer_reward if the proposer includes 100% of precommits.
- `withdraw_addr_enabled:` If true, delegators can set a different address to withdraw their rewards. Set to false if you want to disable transfers at genesis, as it can be used as a way to get around the restriction.
- `delegator_withdraw_infos:` List of delegators withdraw address. Generally null if genesis was not exported from previous state.
- `previous_proposer:` Proposer of the previous block. Set to "" if genesis was not exported from previous state.
- `outstanding_rewards:` Outstanding (un-withdrawn) rewards. Set to null if genesis was not exported from previous state.
- `validator_accumulated_commission:` Outstanding (un-withdrawn) commission of validators. Set to null if genesis was not exported from previous state.
- `validator_historical_rewards:` Set of information related to the historical rewards of validators and used by the distribution module for various computation. Set to null if genesis was not exported from previous state.
- `validators_current_rewards:` Set of information related to the current rewards of validators and used by the distribution module for various computation. Set to null if genesis was not exported from previous state.
- `delegator_starting_infos:` Tracks the previous validator period, the delegation's amount of staking token, and the creation height (to check later on if any slashes have occurred). Set to null if genesis was not exported from previous state.
- `validator_slash_events:` Set of information related to the past slashing of validators. Set to null if genesis was not exported from previous state.

### **Governance**

The gov module handles all governance-related transactions. The initial state of the gov section looks like the following:

Let us break down the parameters:

- `starting_proposal_id:` This parameter defines the ID of the first proposal. Each proposal is identified by a unique ID.
- `deposits:` List of deposits for each proposal ID. Set to null if genesis was not exported from previous state.
- `votes:` List of votes for each proposal ID. Set to null if genesis was not exported from previous state.
- `proposals:` List of proposals for each proposal ID: Set to null if genesis was not exported from previous state.
- `deposit_params`
  - `min_deposit:` The minimum deposit required for the proposal to enter Voting Period. If multiple denoms are provided, the OR operator applies.
  - `max_deposit_period:` The maximum period (in
- voting_params
  - `voting_period:` Length of the voting period in
- tally_params
  - `quorum:` Minimum percentage of bonded staking tokens that needs to vote for the result to be valid.
  - `threshold:` Minimum percentage of votes that need to be YES for the result to be valid.
  - `veto:` Maximum percentage NO_WITH_VETO votes for the result to be valid.
  - `governance_penalty:` Penalty for validators that do not vote on a given proposal.

### **Slashing**

The slashing module handles the logic to slash delegators if their validator misbehave. The slashing section in genesis looks as follows:

Let us break down the parameters:

- params
  - `max_evidence_age:` Maximum age of the evidence in
  - `signed_blocks_window:` Moving window of blocks to figure out offline validators.
  - `min_signed_per_window:` Minimum percentage of precommitsthat must be present in the block window for the validator to be considered online.
  - `downtime_jail_duration:` Duration in
  - `slash_fraction_double_sign:` Percentage of delegators bonded stake slashed when their validator double signs.
  - `slash_fraction_downtime:` Percentage of delegators bonded stake slashed when their validator is down.
- `signing_infos:` Various infos per validator needed by the slashing module. Set to {} if genesis was not exported from previous state.
- `missed_blocks:` Various infos related to missed blocks needed by the slashing module. Set to {} if genesis was not exported from previous state.

### **Genesis Transactions**

By default, the genesis file do not contain any gentxs. A gentx is a transaction that bonds staking token present in the genesis file under accounts to a validator, essentially creating a validator at genesis. The chain will start as soon as more than 2/3rds of the validators (weighted by voting power) that are the recp of a valid gentx come online after genesis_time.

A gentx can be added manually to the genesis file, or via the following command:

Copy `terpd collect-gentxs`

This command will add all the gentxs stored in \~/.terp/config/gentx to the genesis file. Learn how to [create a genesis transaction](https://hub.cosmos.network/main/validators/validator-setup.html#participate-in-genesis-as-a-validator).

## Current Proposed Genesis Parameters Configuration

```json
{
    "genesis_time": "2022-07-10T00:00:00.0000000Z",
    "chain_id": "testing",
    "initial_height": "1",
    "consensus_params": {
      "block": {
        "max_bytes": "22020096",
        "max_gas": "-1",
        "time_iota_ms": "1000"
      },
      "evidence": {
        "max_age_num_blocks": "100000",
        "max_age_duration": "172800000000000",
        "max_bytes": "1048576"
      },
      "validator": {
        "pub_key_types": [
          "ed25519"
        ]
      },
      "version": {}
    },
    "app_hash": "",
    "app_state": {
      "auth": {
        "params": {
          "max_memo_characters": "256",
          "tx_sig_limit": "7",
          "tx_size_cost_per_byte": "10",
          "sig_verify_cost_ed25519": "590",
          "sig_verify_cost_secp256k1": "1000"
        },
        "accounts": []
      },
      "authz": {
        "authorization": []
      },
      "bank": {
        "params": {
          "send_enabled": [],
          "default_send_enabled": false
        },
        "balances": [],
        "supply": [],
        "denom_metadata": []
      },
      "capability": {
        "index": "1",
        "owners": []
      },
      "crisis": {
        "constant_fee": {
          "denom": "uterp",
          "amount": "1000"
        }
      },
      "distribution": {
        "params": {
          "community_tax": "0.020000000000000000",
          "base_proposer_reward": "0.010000000000000000",
          "bonus_proposer_reward": "0.040000000000000000",
          "withdraw_addr_enabled": true
        },
        "fee_pool": {
          "community_pool": []
        },
        "delegator_withdraw_infos": [],
        "previous_proposer": "",
        "outstanding_rewards": [],
        "validator_accumulated_commissions": [],
        "validator_historical_rewards": [],
        "validator_current_rewards": [],
        "delegator_starting_infos": [],
        "validator_slash_events": []
      },
      "evidence": {
        "evidence": []
      },
      "feegrant": {
        "allowances": []
      },
      "feeibc": {
        "identified_fees": [],
        "fee_enabled_channels": [],
        "registered_payees": [],
        "registered_counterparty_payees": [],
        "forward_relayers": []
      },
      "genutil": {
        "gen_txs": []
      },
      "gov": {
        "starting_proposal_id": "1",
        "deposits": [],
        "votes": [],
        "proposals": [],
        "deposit_params": {
          "min_deposit": [
            {
              "denom": "uterp",
              "amount": "710000000"
            }
          ],
          "max_deposit_period": "1440s"
        },
        "voting_params": {
          "voting_period": "1440s"
        },
        "tally_params": {
          "quorum": "0.334000000000000000",
          "threshold": "0.500000000000000000",
          "veto_threshold": "0.334000000000000000"
        }
      },
      "ibc": {
        "client_genesis": {
          "clients": [],
          "clients_consensus": [],
          "clients_metadata": [],
          "params": {
            "allowed_clients": [
              "06-solomachine",
              "07-tendermint"
            ]
          },
          "create_localhost": false,
          "next_client_sequence": "0"
        },
        "connection_genesis": {
          "connections": [],
          "client_connection_paths": [],
          "next_connection_sequence": "0",
          "params": {
            "max_expected_time_per_block": "30000000000"
          }
        },
        "channel_genesis": {
          "channels": [],
          "acknowledgements": [],
          "commitments": [],
          "receipts": [],
          "send_sequences": [],
          "recv_sequences": [],
          "ack_sequences": [],
          "next_channel_sequence": "0"
        }
      },
      "interchainaccounts": {
        "controller_genesis_state": {
          "active_channels": [],
          "interchain_accounts": [],
          "ports": [],
          "params": {
            "controller_enabled": true
          }
        },
        "host_genesis_state": {
          "active_channels": [],
          "interchain_accounts": [],
          "port": "icahost",
          "params": {
            "host_enabled": true,
            "allow_messages": []
          }
        }
      },
      "intertx": null,
      "mint": {
        "minter": {
          "inflation": "0.130000000000000000",
          "annual_provisions": "0.000000000000000000"
        },
        "params": {
          "mint_denom": "upersyx",
          "inflation_rate_change": "0.130000000000000000",
          "inflation_max": "0.200000000000000000",
          "inflation_min": "0.070000000000000000",
          "goal_bonded": "0.000710000000000000",
          "blocks_per_year": "6311520"
        }
      },
      "params": null,
      "slashing": {
        "params": {
          "signed_blocks_window": "1420",
          "min_signed_per_window": "0.242000000000000000",
          "downtime_jail_duration": "600s",
          "slash_fraction_double_sign": "0.050000000000000000",
          "slash_fraction_downtime": "0.001000000000000000"
        },
        "signing_infos": [],
        "missed_blocks": []
      },
      "staking": {
        "params": {
          "unbonding_time": "1814400s",
          "max_validators": 100,
          "max_entries": 7,
          "historical_entries": 10000,
          "bond_denom": "uterpx"
        },
        "last_total_power": "0",
        "last_validator_powers": [],
        "validators": [],
        "delegations": [],
        "unbonding_delegations": [],
        "redelegations": [],
        "exported": false
      },
      "terp": {
        "params": {},
        "terpidList": [],
        "terpidCount": "0",
        "supplychainList": [],
        "supplychainCount": "0"
      },
      "transfer": {
        "port_id": "transfer",
        "denom_traces": [],
        "params": {
          "send_enabled": true,
          "receive_enabled": true
        }
      },
      "upgrade": {},
      "vesting": {},
      "wasm": {
        "params": {
          "code_upload_access": {
            "permission": "Nobody",
            "address": "",
            "addresses": []
          },
          "instantiate_default_permission": "Nobody"
        },
        "codes": [],
        "contracts": [],
        "sequences": []
      }
    }
  }
  ```

### Genesis File Creation Process

The steps to recreate the genesis file is as follows:

- Export [Cosmos Hub](https://github.com/cosmos/testnets/blob/master/public/UPGRADES.md)
- Export BCNA snapshot
- Extract addresses of both exports via [the drop tooling](https://github.com/da0-da0/drop)
- Extract holders of Scavenger Hunt
- Extract holders of Terp OG Challenges
- Add Treasury Depositor
- Calculate [points](../points/README.md) for each genesis address
- Add genesis accounts to genesis.json
- Add all validator gentxs
