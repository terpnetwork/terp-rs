# Genesis Distribution Patch

It has been determined that the genesis distribution proposed and deployed to morocco-1 was not accurately distributed. The intended distribution ratio was the following:

- Cosmos Hub Holders:  `6.11 % of 420 Million TERP`
- Bitcanna Holders:  `1.911 % of 420 Million TERP`
- Terp Og participants: `53,508 TERP`
- Scavenger Hunt participants: `53,508 TERP`
- Terp Network Foundation: `16% of 420 Million TERP`

> However, our analysis shows that ultimately the inital distribution fot Cosmos Hub & Bitcanna holders resulted in more closer to `7.04%` of the total supply.

There are now scripts located in this repository that can calucate and verify the accurate distribution rate for our genesis.
This will lets anyone determe the additional allocations needed to respect the original communication during geneis.

Some wallet addresses were even allocated more than 'what they should have recieved', with a maximum excess for a wallet being 3.2K tokens more than communicated. We will NOT propose to clawback these tokens, but rather compensate by burning the total excess from the foundation DAOs allocations.

## Step 1: Patch Genesis Distribution Allocation

The following script runs the entire workflow to generate an accurate representation of the genesis distribution for Terp Network.

```sh
node main.js -1
```

We have also separated each step to manually run for debugging purposes

### A. Prepare Accurate Allocations

```sh
node main.js -2
```

Will take the snapshot distributions `gaia.csv` & `bcna_delegators.csv`, which were used to calculate the inital genesis distribution for Terp Network, and create a overview of points & token allocation for these projects, printing the results into `scripts-data/final-output.csv`.

**This is our basis for determining what is an accurate distribution.**

A `points-distribution.csv` file will be generated during this function, which displays the points allocated for each projects percentile range. Also, a `total-points.csv` file is generated that gives us a count of the total points, tokens per point based on desired % of supply, and the total amount of tokens actually to be distributed.

#### Interchain Supporter Multiplier

Included in this calculation are points allocated to those address that are found in overlaps between eligible projects. These also have a piecewise linear function, however unlike the individual projects tokens-per-points (which are static throughout the percentile ranges), these percentile ranges have dynamic token per point allocations.

### B. Analyze genesis file & `morocco-1` export

We can obtain an export of the data from a full node of Terp Network via `terpd export`. This will let us then compare the discrepencies from what acutally was distributed, to what should be distributed. Run the following:

```sh
node main.js -3
```

This will take the exported state, and sort the accounts by whether or not they have submitted atleast 1 transaction on-chain, `accounts-active.json` & `accounts-inactive.json`. This can let us create additional rewards for active network participants since the time of genesis, and also calculate the differences between the original amount allocated on block height 1 of Terp Network, with our new more accurate calculation.

### C. Confirm Difference Between Expected And Acutal Allocation

```sh
node main.js -4
```

Calcualtes accurate allocations, flagging discrepancies between expected and original values for logging these descrepancies.

### D. Summarize Results

Creates the summary files highlighting changes between updated,accurate distribution and previous inaccurate distribution.

```sh
node main.js -5
```
