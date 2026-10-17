import fs from 'fs';

import { processSacNFTdata, encodeAddrs } from "./solana-utils.js";
import { processGenesisState, calculateTokenDifference, summarizeAllResults, summarizeScavengerHunt } from './exported-state.js';
import { processHeadstashDistributions, generateMerkleInput } from './headstash-scripts.js';
import { processGenesisDistribution, checkAddresses } from './genesis-script.js';
import { fairPercentileRanges, applyNormalizationToAllProjects, generateOverviewReadme } from './calculations.js'
import { SAC_ENCODED_FILE, SAC_JSON_PATH, HEADSTASH_YAML, ETH_RPC_URL } from './constants.js';
import { readYamlFile } from './utils.js'
import { ethers } from 'ethers';
import { determineAllPubkeys, runPubkeyScrapeCli } from './pubkeys.js'

// Process command line arguments
const args = process.argv.slice(2);
if (args.length < 1) {
    console.error('Invalid option.');
} else if (args[0] === '-1') {
    processGenesisDistribution().catch(console.error);
    processGenesisState();
    checkAddresses();
    calculateTokenDifference();
    summarizeAllResults();
} else if (args[0] === '-2') {
    processGenesisDistribution().catch(console.error);
} else if (args[0] === '-3') {
    processGenesisState();
} else if (args[0] === '-4') {
    checkAddresses();
} else if (args[0] === '-5') {
    calculateTokenDifference();
} else if (args[0] === '-6') {
    summarizeAllResults();
    summarizeScavengerHunt();
} else if (args[0] === '-hs') {
    const data = await readYamlFile(HEADSTASH_YAML);
    await fairPercentileRanges(data);
    await applyNormalizationToAllProjects();
    await processSacNFTdata(SAC_JSON_PATH);
    await encodeAddrs(SAC_ENCODED_FILE, SAC_ENCODED_FILE);
    await processHeadstashDistributions(HEADSTASH_YAML).catch(console.error);
} else if (args[0] === '-7') {
    await processHeadstashDistributions(HEADSTASH_YAML).catch(console.error);
} else if (args[0] === '-8') {
    applyNormalizationToAllProjects();
} else if (args[0] === '-9') {
    // reads json of solana NFT holder snapshot, creates csv with # of tokens unique addrs hold
    await processSacNFTdata(SAC_JSON_PATH);
    await encodeAddrs(SAC_ENCODED_FILE, SAC_ENCODED_FILE);
} else if (args[0] === '-10') {
    generateMerkleInput()
} else if (args[0] === '-11') {
    generateOverviewReadme()
} else if (args[0] === '-12') {
    // Self-hosted geth *or* public-good RPC pool (ETH_RPC_MODE=auto|local|public).
    if (process.env.ETH_RPC_MODE === 'local' && !process.env.ETH_RPC_URLS) {
        const provider = new ethers.JsonRpcProvider(ETH_RPC_URL);
        await determineAllPubkeys(provider);
    } else {
        await runPubkeyScrapeCli();
    }
} else {
    console.error('Invalid option.');
}