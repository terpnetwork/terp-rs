# Playbook: contract escrow expired

**Audience:** support systems, contract operators  
**Path:** B (callbacks / claim contracts)  
**Related:** `x/hashmerchant/spec/08_parameters.md`, [support role](../../../docs/hashmerchant/support-role.md)

## Symptoms

- Contract stops receiving hashmerchant sudo callbacks
- `terpd q hashmerchant escrow <contract>` shows expired / `paid_until` past current height
- Roots still confirm on-chain for the chain_uid, but this contract is silent

## Quick checks

```bash
terpd q hashmerchant escrow <contract-addr>
terpd q hashmerchant params   # min_escrow_amount, prune_interval, escrow_denom
terpd status | jq -r '.sync_info.latest_block_height'
```

## Actions

1. Query escrow: `terpd q hashmerchant escrow <contract>`
2. Refill with accepted denom (default `uterp`):

   ```bash
   terpd tx hashmerchant refill-escrow <contract-addr> <amount> \
     --from <operator> --gas auto -y
   ```

3. Confirm `paid_until` (or equivalent) extends past current height.
4. Wait for the next confirmed root + sudo delivery.
5. If never registered: use `register-contract` with escrow (not only refill).

## Prevention

- Monitor `paid_until` vs current height in support runbooks
- Budget refill for every registered claim / demo contract
- Remember: `paid_blocks ≈ (amount / min_escrow_amount) * prune_interval` (see module params)

## Do not

- Expect callbacks after escrow expiry “for free”
- Confuse bank balance of the contract with module escrow balance

## Exit criteria

- Escrow query shows active coverage through a future height
- Next root confirmation delivers sudo (or lab smoke verifies receipt)
