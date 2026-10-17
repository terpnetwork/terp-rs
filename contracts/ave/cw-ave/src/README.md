

Network | code-id | contract-addr ||
--- | --- | --- | --- | 
Stargaze `elgafar-1` | - | 
Osmosis `osmosis-test-5` | 11043 | osmo10jsnt4rhfsr7w50z3vg3ghfxy98fassnsxnmdypfuvnzzhscsegqsf9432


## Scripts
build contract
```sh
  docker run --rm -t -v "$(pwd)":/code \
    --mount type=volume,source="$(basename "$(pwd)")_cache",target=/code/target \
    --mount type=volume,source=registry_cache,target=/usr/local/cargo/registry \
    cosmwasm/optimizer-arm64:0.16.0
```

## Create A New AvEvent
```json
{
    
}
```

## Purchase AvEvent Ticket
```json
{
    "shit_strap": {
        "shit": {
            "denom": {"native": "your_denom_here"},
            "amount": "69"
        }
    }
}
```
 