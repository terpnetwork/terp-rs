
## Current Infusion Minters

<div align="center">

[![preview](../../public/peaceful-nightsky.png)]()\
*Art By [Jinxto](https://www.stargaze.zone/p/jinxto/collections), generated with go installed followed by running the following:\
`cd art/go/ascii-image-converter && go build && ./ascii-image-converter ../../../public/peaceful-nightsky.png -C`*
</div>


`stargaze-1`:
- stars1333zgwvcxe04apsg98mccpc2fg7ft5xwl9ewkey2fwgf70zghsrse5nglu
- stars1zkdqlly53sdafh6dhcpuapxxc3llxyqw4v9ekk9x553mc4mv0xlqkyvg3l
- stars16k2ewvfapjdsnncdk2snv9wj3f8vg3j82sfq962906rdx3n67kns22fsvh
- current code-id: 682

## Additional Info 

###  Infused Collection
#### Token IDs
For each infusion, a new infused collection is created. These token-id's are incremented from 0, and kept track of the next token id in the infuser contract. 

#### Base-URI
The base uri is the folder stored to ipfs containing a list of ipfs documents. The contract sets the uri for each new token being minted based on the count:
This requireds the base uri to be provided with the format of `ipfs://abcd`


### State.json
The deployed contracts state can be found [here](./state.json). This file is generated from making use of cw-orchestrator scripts. 

### Creation Fees 
A minimum creation fee may be set on the contract level, requiring a fee in order to create an new infusion. This fee goes to the admin of the contract.

### Infusion Fees 
A minimum fee required to infuse may be set on the contract level, requiring all unique infusions for a contract to have at least this fee and token set as eligible fee.
 

## GOALS: 
- add documentation
- create infusion minter contract
- cross-chain burn-to-mint via polytone
- add pagination for querying infusions
- max mint limit for each eligible collection
- support for depositing nfts (used to ommit nfts approval tx requirement)
- immuatbility support (cannot update baseuri, ...)
- add bundle recipies: configure what types of bundles can be made:
- community oriented: only holders of collections may set them as eligible to be burnt
  <!-- - allOf: requires the minimum for all eligible collections -->
  <!-- - anyOf: any of 1 of the eligible collections -->
  - anyOfBlend: select which elgible collections may be set to have blended requirements.
- instantiate fee-split upon infusion creation for burnt collecction royalty recipients
-  Infusion Factories 
  - open-edition factory/minter
  - randomized token-id, open-edition factory/minter
  - multi-chain factory/minter
- infused nft metadata: embed data of tokens infused when minting an nft from infused collections
- feesub - correctly handle eligible collections with identical feesub payment tokens


