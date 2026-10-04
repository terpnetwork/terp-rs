# Terp721-Account Manifold

Minter and Marketplace for Terp Accounts

## Instantiate

| Parameter | Description |
| --- | --- |
| `admin` | Temporary admin for managing whitelists (optional). |
| `verifier` | Oracle for verifying text records (optional). |
| `collection_code_id` | Code ID for BS721-Account. Used to instantiate a new account collection. |
| `marketplace_addr` | Address of the BS721-Account marketplace. |
| `min_account_length` | Minimum length an account ID can be. |
| `max_account_length` | Maximum length an account ID can be. |
| `base_price` | Base price for an account. Used to calculate premium for small account accounts. |

## Actions

| Actions |
| --- |
| `MintAndList` |
| `Pause` |
| `UpdateConfig` |

## Queries

| Contract Queries | Description |
| --- | --- |
| `Ownership` | Get the ownership information of the contract. |
| `Collection` | Get the collection address associated with the contract. |
| `Params` | Get the sudo parameters of the contract. |
| `Config` | Get the configuration of the contract. |

## Sudo Parameters

| Sudo Parameters |
| --- |
| `UpdateParams` |
| `UpdateAccountCollection` |
| `UpdateAccountMarketplace` |
