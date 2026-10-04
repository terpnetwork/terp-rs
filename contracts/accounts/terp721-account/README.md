# Terp721-Account



 ## Instantiate Msg 
| Parameter | Description | 
| --- | --- | 
| `verifier` | Verification oracle address (optional). | 
| `marketplace` | Address of the marketplace contract. | 
| `base_init_msg` | Base initialization message for the BS721 contract. |

## BS721 Accounts Actions 
|   |  |
| --- |--- |
| `AssociateAddress` | |
| `UpdateMyReverseMapKey` |  Each account can create a mapping of bech32 addresses for non type 639 coin chains (Terp defaults to 639). Uses the external address as the key, and saves the users Canonical Addr to the store for future retrival via `transcode()`. |
| `UpdateImageNft` | 
| `AddTextRecord` |     
| `RemoveTextRecord` | 
| `UpdateTextRecord` | 
| `VerifyTextRecord` |

## Admin Only 
| Admin Only | 
| --- | 
| `UpdateVerifier` | 
| `Set Marketplace` | 
| `FreezeCollectionInfo` |
 

## Actions and Descriptions
| Cw721-Standard | 
| --- | 
| `Approve` | 
| `ApproveAll` | 
| `Revoke` | 
| `RevokeAll` | 
| `Mint` | 
| `Burn` |

## Minting Workflow 
| Cw721-Standard | 
| --- | 
| `TransferNft` | 
| `SendNft` |
 

## BS721 Accounts Queries 
| Query | Description | 
| --- | --- | 
| `Params` | Returns the sudo parameters. | 
| `Account` | Reverse lookup of account for a given address. | 
| `AccountMarketplace` | Returns the marketplace contract address. | 
| `AssociatedAddress` | Returns the associated address for a given account. This is the address set during `ExecuteMsg::AssociateAddress`,contract expects this to be the abstract-account contract address if a token is being used for tokenized account | 
| `ImageNFT` | Returns the image NFT for a given account. | 
| `TextRecords` | Returns the text records for a given account. | 
| `IsTwitterVerified` | Returns whether Twitter is verified for a given account. | 
| `Verifier` | Returns the verification oracle address. | 
| `OwnerOf` | Returns the owner of a specific token. | 
| `Approval` | Returns the approval status for a specific token and spender. | 
| `Approvals` | Returns all approvals for a specific token. | 
| `AllOperators` | Returns all operators for a specific owner. | 
| `NumTokens` | Returns the total number of tokens. | 
| `ContractInfo` | Returns information about the contract. | 
| `NftInfo` | Returns information about a specific NFT. | 
| `AllNftInfo` | Returns all information about a specific NFT. | 
| `Tokens` | Returns a list of tokens owned by a specific owner. | 
| `AllTokens` | Returns a list of all tokens. | 
| `Minter` | Returns the ownership information of the minter. |
