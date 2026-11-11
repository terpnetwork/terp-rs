# Harmony-API

## Goal
The goal is to have this API:
- A. Fetch & pull updates to the DAO-DAO smart contract repo, daily.
- B. Tally and record the file & line of every time the word `DAO` exists.
- C. Associate `DAO` instances in file paths with a token-id
- D. Calculate any changes of tally in `DAO` instances for each file, daily.
  - if file has less count than yesterday, randomly choose token-id associated with a file to update metadata
  - if file has more count than yesterday, get the latest token-id, and increase by the difference. (also associate token-id with file)
- E. With access to the wallet that is admin of the collection, broadcast msgs to either:
  - update metadata for token-id's that were randomly selected to be transformed into its second half.
  - mint new token-id, and send to DAO.
- F. allow purchasing of new dao-harmony tokens via custom smart contract



## Folders included in tally
- `./dao-contracts/contracts`
- `./dao-contracts/packages`
- `./dao-contracts/scripts`
