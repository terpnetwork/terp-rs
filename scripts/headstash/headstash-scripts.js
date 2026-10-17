// Headstash Scripts that creates a single file from each community distribution csv.
// 1. calculate points for each community
// 2. identify and merge any address that exist in multiple community distributions 
// 3. if solana wallet, base64 encode wallet address 
import fs from 'fs';
import { fileURLToPath } from 'url';
import path, { dirname } from 'path'
import { applyNormalizationToAllProjects } from './calculations.js';
import { readCsvFile, readYamlFile } from './utils.js';
import { HEADSTASH_FINAL_TALLY, SINSEMILLA_JSON_FILE } from './constants.js'
const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
// step 1: determine point distribution for each communinty
// step 2: determine tokens to allocate for address based on tpp  
// step 3: check for reoccurring addresses between all communnities. if addr exists, sum together points allocated.
// step 4: if address is not eth address, we need to base64 encode the address (as it is a solana public address)
// step 5: create new 1 new csv with final tally 
function determinePointDistribution(pointsConfig, walletAmount) {
    // Ensure walletAmount is a number
    const amount = parseFloat(walletAmount) || 0;

    // Get cutoffs (these are actual token balances)
    const threeCut = parseFloat(pointsConfig.threePointsUpTo.cutoffAmount) || 0;
    const twoCut = parseFloat(pointsConfig.twoPointsUpTo.cutoffAmount) || 0;

    // Compare balance directly
    if (amount >= threeCut) {
        return 3;
    } else if (amount >= twoCut) {
        return 2;
    } else if (amount > 0) {
        return 1;
    }
    return 0;
}

function isEthereumAddress(address) {
    return address.length === 42 && address.startsWith('0x');
}

function encodeSolanaAddress(address) {
    return Buffer.from(address, 'utf-8').toString('base64');
}

// Create an object to store the final tally
let finalTally = {};

async function processHeadstashDistributions(yamlFile) {
    let addressCommunities = {};
    let communities = [];

    const data = await readYamlFile(yamlFile);
    // create percentile ranges
    // await fairPercentileRanges(data);
    // await applyNormalizationToAllProjects();

    for (let project of data.projects) {
        try {
            // Read the CSV file for the current community
            const csvData = await readCsvFile(project.csv);
            console.log(`Processing CSV for: ${project.name}`)
            // Process the CSV data
            csvData.forEach((row) => {
                // Get the address and amount from the current row
                let address = row.addr;
                let amount = parseInt(row.amount);
                let points = determinePointDistribution(project.points, amount);
                let tokens = points * project.tpp;
                const microTokens = Math.floor(tokens * 1_000_000); // integer in micro-denom
                if (!isEthereumAddress(address)) {
                    address = encodeSolanaAddress(address);
                }
                console.log(`addr ${address}`)
                console.log(`amount ${amount}`, amount)
                console.log(`points ${points}`, points)
                console.log(`tokens ${microTokens}`)
                // Add the tokens to the final tally
                if (address in finalTally) {
                    finalTally[address] += microTokens;
                } else {
                    finalTally[address] = microTokens;
                }

                // Add the community to the address's communities
                if (!addressCommunities[address]) {
                    addressCommunities[address] = {};
                }
                if (!addressCommunities[address][project.csv]) {
                    addressCommunities[address][project.csv] = 0;
                }
                addressCommunities[address][project.csv] += points;

                // Add the community to the list of communities
                if (!communities.includes(project.csv)) {
                    communities.push(project.csv);
                }
            });
        } catch (error) {
            console.error(`Error processing project: ${error}`);
        }
    }

    // Create a new CSV file with the final tally
    try {
        console.log('Creating Final tally CSV...');
        await createFinalTallyCsv(finalTally, addressCommunities, communities);
        console.log('Final tally CSV file created successfully!');
    } catch (error) {
        console.error(`Error creating final tally CSV: ${error}`);
    }
}



// Function to create the final tally CSV file
function createFinalTallyCsv(finalTally, addressCommunities, communities) {
    return new Promise(async (resolve, reject) => {
        let csvContent = "addr,allocation";
        for (let community of communities) {
            csvContent += `,${path.basename(community)}`;
        }
        csvContent += "\n";

        Object.keys(finalTally).sort((a, b) => finalTally[b] - finalTally[a]).forEach((address) => {
            let row = `${address},${finalTally[address]}`;
            for (let community of communities) {
                if (addressCommunities[address] && addressCommunities[address][community]) {
                    row += `,${addressCommunities[address][community]}`;
                } else {
                    row += ",0";
                }
            }
            csvContent += row + "\n";
        });

        fs.writeFile(HEADSTASH_FINAL_TALLY, csvContent, (err) => {
            if (err) {
                reject(err);
            } else {
                resolve();
            }
        });

        try {
            console.log('Creating Community points summary CSV...');
            await createCommunityPointsSummaryCsv(addressCommunities, communities);
            console.log('Community points summary CSV file created successfully!');
        } catch (error) {
            console.error(`Error creating community points summary CSV: ${error}`);
        }
    });
}

// Function to create the community points summary CSV file
function createCommunityPointsSummaryCsv(addressCommunities, communities) {
    return new Promise((resolve, reject) => {
        let communityPoints = {};

        // Calculate the sum of points for each community
        communities.forEach((community) => {
            communityPoints[community] = { points: {}, addrCount: 0 };
            Object.keys(addressCommunities).forEach((address) => {
                if (addressCommunities[address][community]) {
                    communityPoints[community].addrCount++;
                    const points = addressCommunities[address][community];
                    if (communityPoints[community].points[points]) {
                        communityPoints[community].points[points]++;
                    } else {
                        communityPoints[community].points[points] = 1;
                    }
                }
            });
        });

        // Create the CSV content
        let csvContent = "community,addrCount,points,count\n";
        communities.forEach((community) => {
            Object.keys(communityPoints[community].points).forEach((points) => {
                csvContent += `${path.basename(community)},${communityPoints[community].addrCount},${points},${communityPoints[community].points[points]}\n`;
            });
        });

        // Write the CSV file
        fs.writeFile('../headstash/scripts-data/community_points_summary.csv', csvContent, (err) => {
            if (err) {
                reject(err);
            } else {
                resolve();
            }
        });
    });
}

/**
 * Generates Sinsemilla-compatible merkle tree input
 * - Reads final_tally.csv (with 'addr' and 'allocation' columns)
 * - Assigns equal amounts of uterp and uthiol = allocation value
 */
const generateMerkleInput = async () => {
    const csvPath = path.join(__dirname, HEADSTASH_FINAL_TALLY);
    const outputPath = path.join(__dirname, SINSEMILLA_JSON_FILE);

    try {
        const rows = await readCsvFile(csvPath);
        console.log(`✅ Loaded ${rows.length} rows from ${csvPath}`);

        const result = {};

        for (const row of rows) {
            const address = row.addr?.trim();
            const allocation = row.allocation?.trim();

            if (!address) {
                console.warn(`⚠️ Missing address, skipping row:`, row);
                continue;
            }

            const amount = allocation && !isNaN(allocation) ? allocation : '0';

            if (amount === '0') {
                // Optional: skip zero allocations
                // Or include them with 0 amount
                console.log(`➡️ Address ${address} has 0 allocation`);
            }

            // TODO: allow defining tokens and their amounts
            result[address] = [
                {
                    name: "uterp",
                    amount // This is a string
                },
                {
                    name: "uthiol",
                    amount // Same amount for uthiol
                }
            ];
        }
        // Sort addresses lexicographically for deterministic order
        const sortedResult = {};
        Object.keys(result)
            .sort()
            .forEach((key) => {
                sortedResult[key] = result[key];
            });

        // Write output
        fs.writeFileSync(outputPath, JSON.stringify(result, null, 2), 'utf8');
        console.log(`✅ Merkle input written to ${outputPath}`);
        console.log(`💡 File ready for Sinsemilla merkle tree generation`);

        return result;
    } catch (error) {
        console.error(`❌ Error generating merkle input:`, error.message);
        throw error;
    }
};

export { processHeadstashDistributions, generateMerkleInput }